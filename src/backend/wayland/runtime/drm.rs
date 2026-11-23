//! DRM/KMS bare-metal backend for running directly on hardware.

use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::drm::DrmEvent;
use smithay::backend::libinput::LibinputInputBackend;
use smithay::backend::session::Event as SessionEvent;
use smithay::backend::session::Session;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::udev::{UdevBackend, UdevEvent};
use smithay::reexports::input::Libinput;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::process::exit;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::init::drm::{GpuResources, init_gpu};
use crate::backend::wayland::render::cursor_theme::CursorManager;
use crate::backend::wayland::render::drm::{
    DrmDeviceId, ScanoutId, build_output_surfaces, create_output_manager,
};

mod devices;
mod frames;
mod layout;
mod output_config;
mod scheduling;
mod session;
#[cfg(test)]
mod tests;
mod topology;
mod vrr;

use devices::{DrmDevices, PRIMARY_DEVICE};
use layout::{compute_total_dimensions, init_layout_state};
use scheduling::DrmLoopState;
use session::{DrmOutputRuntime, DrmSession};

#[derive(Debug, Clone, Copy)]
enum DrmRuntimeEvent {
    SessionPaused,
    SessionActivated,
    VBlank(ScanoutId),
    PointerMoved { old_location: crate::types::Point },
    OutputTopologyChanged,
    DeviceRemoved(libc::dev_t),
}

// WARNING: This function is extremely fragile, do not refactor or mess with it without
// great care and patience for random ass segfaults. Yes, this is awful, leave it.
// Hours spent on this: ~3h
pub fn run() -> ! {
    log::info!("Starting DRM/KMS backend");
    let wm = super::bootstrap::create_wayland_wm();
    let (event_loop, mut state) = crate::backend::wayland::compositor::new_event_loop_and_state(wm);

    let loop_handle = event_loop.handle();

    let (mut session, notifier) = LibSeatSession::new().expect("libseat session");
    let seat_name = session.seat();
    log::info!("Session on seat: {seat_name}");

    state.native.runtime.session = Some(session.clone());

    crate::keyboard_layout::init_keyboard_layout(&mut state.ctx());

    let GpuResources {
        primary_gpu_path,
        drm_device,
        drm_notifier,
        drm_fd,
        gbm_device,
        egl_display,
        mut renderer,
    } = init_gpu(&mut session, &seat_name);
    log::info!("Using GPU: {:?}", primary_gpu_path);

    state.init_drm_syncobj(drm_fd.clone());

    let cursor_manager = CursorManager::from_environment(&state.native.cursor_config);
    // The scanout device may be a different GPU from the render device (split
    // setups).  Open its own GBM handle for framebuffer import/export.
    let scanout_gbm = GbmDevice::new(drm_fd.clone()).expect("scanout GbmDevice::new");
    let mut output_manager =
        create_output_manager(drm_device, &renderer, &gbm_device, &scanout_gbm);

    let output_surfaces = {
        build_output_surfaces(
            &mut output_manager,
            PRIMARY_DEVICE,
            &mut renderer,
            &mut state.native,
        )
    };
    let devices = Rc::new(RefCell::new(DrmDevices::new(
        primary_gpu_path,
        output_manager,
        gbm_device,
        session.clone(),
        seat_name.clone(),
    )));
    for entry in &output_surfaces {
        state
            .native
            .space
            .map_output(&entry.output, (entry.rect.x, entry.rect.y));
    }

    // Register all DRM outputs with wlr-output-management.
    state
        .native
        .output_management_state
        .add_heads::<crate::backend::wayland::compositor::WaylandState>(
            output_surfaces.iter().map(|e| &e.output),
        );

    let layout = compute_total_dimensions(&output_surfaces);

    {
        use crate::monitor::refresh_monitor_layout;
        refresh_monitor_layout(&mut state.ctx());
    }
    state
        .native
        .push_command(crate::backend::wayland::commands::WmCommand::SyncLayerExclusiveZones);
    crate::monitor::apply_monitor_config(&mut state.ctx());

    let layout_state = init_layout_state(&output_surfaces, layout);
    // Calloop dispatches sources and the loop callback sequentially on this
    // thread. Libinput only needs the current layout bounds, so share that
    // small copy without putting the complete layout behind an atomic lock.
    let input_dimensions = Rc::new(Cell::new(layout));
    let mut loop_state = DrmLoopState::new(&output_surfaces);
    let (runtime_event_tx, runtime_event_rx) = mpsc::channel();

    super::bootstrap::setup_listen_socket(&loop_handle, &mut state);

    // Share startup snapshots only between device opening and event dispatch.
    // They are consumed on DeviceAdded, including when libinput resumes.
    let initial_lid_states = Rc::new(std::cell::RefCell::new(HashMap::new()));
    let mut libinput_context = Libinput::new_with_udev(
        crate::backend::wayland::input::drm::LidAwareInputInterface::new(
            session.clone(),
            Rc::clone(&initial_lid_states),
        ),
    );
    libinput_context
        .udev_assign_seat(&seat_name)
        .expect("libinput assign seat");

    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());
    let shared_input_dimensions = Rc::clone(&input_dimensions);
    let runtime_event_tx_input = runtime_event_tx.clone();
    loop_handle
        .insert_source(libinput_backend, move |event, _, state| {
            let layout = shared_input_dimensions.get();
            let initial_lid_state = match &event {
                smithay::backend::input::InputEvent::DeviceAdded { device } => {
                    initial_lid_states.borrow_mut().remove(device.sysname())
                }
                _ => None,
            };

            // The source has exclusive root access for this input event.
            // Shared actions reborrow the owned core before seat callbacks.
            let old_pointer_location = crate::types::Point::new(
                state.native.runtime.pointer_location.x as i32,
                state.native.runtime.pointer_location.y as i32,
            );
            let outcome = {
                crate::backend::wayland::input::drm::dispatch_libinput_event(
                    event,
                    state,
                    layout,
                    initial_lid_state,
                )
            };

            use crate::backend::wayland::input::drm::LibinputEventOutcome;
            match outcome {
                LibinputEventOutcome::Ignored => {}
                LibinputEventOutcome::Activity => state.notify_activity(),
                LibinputEventOutcome::PointerMoved => {
                    state.notify_activity();
                    let _ = runtime_event_tx_input.send(DrmRuntimeEvent::PointerMoved {
                        old_location: old_pointer_location,
                    });
                }
            }
        })
        .expect("failed to insert libinput source");

    setup_session_handlers(
        &loop_handle,
        notifier,
        &mut libinput_context,
        Rc::clone(&devices),
        runtime_event_tx.clone(),
    );

    let token = setup_drm_vblank_handler(
        &loop_handle,
        drm_notifier,
        PRIMARY_DEVICE,
        runtime_event_tx.clone(),
    )
    .expect("primary DRM notifier source");
    devices
        .borrow_mut()
        .devices
        .get_mut(&PRIMARY_DEVICE)
        .unwrap()
        .token = Some(token);
    // Enumerate additional devices at startup, even if udev sent their add
    // events before the compositor started listening.
    loop_state.topology_dirty = true;
    setup_udev_hotplug_handler(&loop_handle, &seat_name, runtime_event_tx.clone());

    let ipc_server = super::bootstrap::autostart_ipc_status_ping(&loop_handle, &mut state.wm);

    // One-shot wakeup for the initial frame. Later render failures use a
    // bounded timer instead of an immediate self-ping loop.
    let (initial_render_ping, initial_render_ping_source) =
        calloop::ping::make_ping().expect("ping");
    event_loop
        .handle()
        .insert_source(initial_render_ping_source, |_, _, _| {})
        .expect("ping source");

    // Compositor redraw pings preserve a target set installed by
    // `request_output_render`.  A bare ping (for example, completion of the
    // asynchronous bar worker) falls back to invalidating all outputs.
    super::bootstrap::setup_render_ping(&loop_handle, &mut state);
    initial_render_ping.ping();

    let start_time = Instant::now();
    let render_failures: HashMap<ScanoutId, u32> = HashMap::new();

    crate::runtime::spawn_status_bar(&mut state.wm);

    let (led_state_tx, led_state_rx) = mpsc::channel();
    state.native.runtime.led_state_tx = Some(led_state_tx);

    super::bootstrap::attach_gles_renderer_and_protocols(
        &mut state,
        crate::backend::wayland::compositor::graphics::Graphics::Drm(Box::new(renderer)),
        Some(&egl_display),
    );

    DrmSession::new(
        DrmOutputRuntime {
            layout_state,
            input_dimensions,
            loop_state,
            output_surfaces,
            devices,
            render_failures,
            cursor_manager,
            start_time,
        },
        runtime_event_tx,
        runtime_event_rx,
        led_state_rx,
        ipc_server,
    )
    .run(event_loop, &mut state);

    state.wm.backend.wallpaper.shutdown();
    crate::startup::autostart::shutdown_autostart();
    crate::backend::wayland::session::stop_graphical_session_target();
    exit(0);
}

fn cursor_size(size: u32) -> u8 {
    size.clamp(1, u8::MAX as u32) as u8
}

/// Compute total screen bounds from output surfaces, origin included.
///
/// Absolute-device mapping needs both the size and the origin: a layout with
/// a negative position (output placed above/left) maps `0..size` onto
/// `origin..origin+size`.
/// Setup session pause/activate handlers for VT switching.
fn setup_session_handlers(
    loop_handle: &calloop::LoopHandle<WaylandState>,
    notifier: smithay::backend::session::libseat::LibSeatSessionNotifier,
    libinput_context: &mut Libinput,
    devices: Rc<RefCell<DrmDevices>>,
    runtime_event_tx: mpsc::Sender<DrmRuntimeEvent>,
) {
    let mut session_libinput = libinput_context.clone();

    loop_handle
        .insert_source(notifier, move |event, _, state| match event {
            SessionEvent::PauseSession => {
                log::info!("Session paused (VT switch away) - suspending rendering");
                {
                    crate::backend::wayland::input::touch::handle_touch_cancel(state);
                }
                session_libinput.suspend();
                for device in devices.borrow_mut().devices.values_mut() {
                    device.manager.pause();
                }
                let _ = runtime_event_tx.send(DrmRuntimeEvent::SessionPaused);
            }
            SessionEvent::ActivateSession => {
                log::info!("Session activated (VT switch back) - resuming rendering");
                if let Err(err) = session_libinput.resume() {
                    log::error!("failed to resume libinput context: {:?}", err);
                }
                for device in devices.borrow_mut().devices.values_mut() {
                    if let Err(error) = device.manager.lock().activate(false) {
                        log::error!(
                            "failed to reactivate DRM device {}: {error}",
                            device.path.display()
                        );
                    }
                }
                let _ = runtime_event_tx.send(DrmRuntimeEvent::SessionActivated);
            }
        })
        .expect("session source");
}

/// Setup DRM vblank handler for render synchronization.
fn setup_drm_vblank_handler(
    loop_handle: &calloop::LoopHandle<WaylandState>,
    drm_notifier: smithay::backend::drm::DrmDeviceNotifier,
    device: DrmDeviceId,
    runtime_event_tx: mpsc::Sender<DrmRuntimeEvent>,
) -> Result<smithay::reexports::calloop::RegistrationToken, anyhow::Error> {
    loop_handle
        .insert_source(drm_notifier, move |event, _metadata, _data| match event {
            DrmEvent::VBlank(crtc) => {
                let _ = runtime_event_tx.send(DrmRuntimeEvent::VBlank(ScanoutId { device, crtc }));
            }
            DrmEvent::Error(err) => {
                log::error!("DRM error: {err}");
            }
        })
        .map_err(|error| anyhow::anyhow!("DRM notifier source: {error}"))
}

/// Wake the DRM runtime whenever udev reports a graphics-device topology
/// change. Connector probing and mutation remain on the compositor thread.
fn setup_udev_hotplug_handler(
    loop_handle: &calloop::LoopHandle<WaylandState>,
    seat_name: &str,
    runtime_event_tx: mpsc::Sender<DrmRuntimeEvent>,
) {
    let backend = match UdevBackend::new(seat_name) {
        Ok(backend) => backend,
        Err(error) => {
            log::error!("failed to monitor DRM hot-plug events: {error}");
            return;
        }
    };
    let retry_handle = loop_handle.clone();
    loop_handle
        .insert_source(backend, move |event, _, _| match event {
            UdevEvent::Added { .. } | UdevEvent::Changed { .. } | UdevEvent::Removed { .. } => {
                if let UdevEvent::Removed { device_id } = event {
                    let _ = runtime_event_tx.send(DrmRuntimeEvent::DeviceRemoved(device_id));
                }
                let _ = runtime_event_tx.send(DrmRuntimeEvent::OutputTopologyChanged);
                // Thunderbolt and DisplayPort MST branches enumerate in
                // stages. Udev does not guarantee that the final connector
                // state has settled when the first DRM change arrives, nor
                // that every later link-state transition produces a distinct
                // event useful to this compositor. Reprobe at bounded settling
                // points so a connector or mode that appears late is not lost
                // until the next physical hotplug.
                for delay in [
                    Duration::from_millis(100),
                    Duration::from_millis(350),
                    Duration::from_secs(1),
                ] {
                    let retry_tx = runtime_event_tx.clone();
                    if let Err(error) = retry_handle.insert_source(
                        smithay::reexports::calloop::timer::Timer::from_duration(delay),
                        move |_, _, _| {
                            let _ = retry_tx.send(DrmRuntimeEvent::OutputTopologyChanged);
                            smithay::reexports::calloop::timer::TimeoutAction::Drop
                        },
                    ) {
                        log::warn!("failed to schedule DRM hot-plug reprobe: {error}");
                    }
                }
            }
        })
        .expect("failed to insert udev hot-plug source");
}
