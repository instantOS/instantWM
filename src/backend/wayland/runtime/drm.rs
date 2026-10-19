//! DRM/KMS bare-metal backend for running directly on hardware.

use smithay::backend::drm::DrmEvent;
use smithay::backend::libinput::LibinputInputBackend;
use smithay::backend::session::Event as SessionEvent;
use smithay::backend::session::Session;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::udev::{UdevBackend, UdevEvent};
use smithay::input::pointer::CursorIcon;
use smithay::reexports::calloop::{EventLoop, LoopSignal};
use smithay::reexports::input::Libinput;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::utils::{Clock, Monotonic};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::mem;
use std::process::exit;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::init::drm::init_gpu;
use crate::backend::wayland::render::cursor::{ResolvedCursor, resolve_cursor};
use crate::backend::wayland::render::cursor_theme::CursorManager;
use crate::backend::wayland::render::drm::{
    DrmDeviceId, OutputSurfaceEntry, ScanoutId, build_output_surfaces, create_output_manager,
};

mod devices;
mod frames;
mod layout;
mod output_config;
mod scheduling;
#[cfg(test)]
mod tests;
mod topology;
mod vrr;

use devices::{DrmDevices, PRIMARY_DEVICE};
use frames::{
    output_refresh, process_commit_redraws, process_cursor_warp, process_frame_callback_requests,
    render_outputs,
};
use layout::{
    DrmLayoutState, compute_total_dimensions, init_layout_state,
    recover_pointer_after_output_change, refresh_drm_layout_state,
};
use output_config::{process_output_configurations, process_output_power_requests};
use scheduling::DrmLoopState;
use topology::reconcile_drm_outputs;

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

    let (
        primary_gpu_path,
        drm_device,
        drm_notifier,
        drm_fd,
        gbm_device,
        egl_display,
        renderer_value,
    ) = init_gpu(&mut session, &seat_name);
    let mut renderer = renderer_value;
    log::info!("Using GPU: {:?}", primary_gpu_path);

    state.init_drm_syncobj(drm_fd.clone());

    let mut cursor_manager = CursorManager::from_environment(&state.native.cursor_config);
    let mut output_manager = create_output_manager(drm_device, &renderer, &gbm_device, &gbm_device);

    let mut output_surfaces = {
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

    let mut layout_state = init_layout_state(&output_surfaces, layout);
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

    let mut ipc_server = super::bootstrap::autostart_ipc_status_ping(&loop_handle, &mut state.wm);

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
    let mut render_failures: HashMap<ScanoutId, u32> = HashMap::new();

    crate::runtime::spawn_status_bar(&mut state.wm);

    let (led_state_tx, led_state_rx) = mpsc::channel();
    state.native.runtime.led_state_tx = Some(led_state_tx);

    super::bootstrap::attach_gles_renderer_and_protocols(
        &mut state,
        crate::backend::wayland::compositor::graphics::Graphics::Drm(Box::new(renderer)),
        Some(&egl_display),
    );

    run_event_loop(
        event_loop,
        &mut state,
        &mut layout_state,
        &input_dimensions,
        &mut loop_state,
        &mut output_surfaces,
        &devices,
        &runtime_event_tx,
        &mut cursor_manager,
        &mut ipc_server,
        &mut render_failures,
        start_time,
        led_state_rx,
        runtime_event_rx,
    );

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

/// Extract a `CursorIcon` from a resolved cursor presentation for
/// animation-timer scheduling.  `Surface` cursors are client-owned and
/// cannot be introspected, so they return `None`.
fn resolved_cursor_icon(p: &ResolvedCursor) -> Option<CursorIcon> {
    match p {
        ResolvedCursor::Hidden | ResolvedCursor::Surface { .. } => None,
        ResolvedCursor::Named(icon) => Some(*icon),
        ResolvedCursor::DndIcon { cursor, .. } => resolved_cursor_icon(cursor),
    }
}

/// Run the main event loop.
///
/// This is the heart of the DRM backend. It handles:
/// - Frame submission tracking
/// - Libinput event dispatch
/// - Layout arrangement
/// - IPC command processing
/// - Window animations
/// - Cursor warp
/// - DRM rendering
#[allow(clippy::too_many_arguments)]
fn run_event_loop(
    mut event_loop: EventLoop<WaylandState>,

    state: &mut WaylandState,
    layout_state: &mut DrmLayoutState,
    input_dimensions: &Rc<Cell<crate::types::Rect>>,
    loop_state: &mut DrmLoopState,
    output_surfaces: &mut Vec<OutputSurfaceEntry>,
    devices: &Rc<RefCell<DrmDevices>>,
    runtime_event_tx: &mpsc::Sender<DrmRuntimeEvent>,
    cursor_manager: &mut CursorManager,
    ipc_server: &mut Option<crate::ipc::IpcServer>,
    render_failures: &mut HashMap<ScanoutId, u32>,
    start_time: Instant,
    led_state_rx: mpsc::Receiver<smithay::input::keyboard::LedState>,
    runtime_event_rx: mpsc::Receiver<DrmRuntimeEvent>,
) {
    let loop_signal: LoopSignal = event_loop.get_signal();
    let loop_handle = event_loop.handle();
    let pointer_handle = state.native.pointer.clone();
    let render_retry_guard = crate::runtime::AnimationTimerGuard::new();
    let shared_input_dimensions = Rc::clone(input_dimensions);
    let monotonic_clock = Clock::<Monotonic>::new();

    event_loop
        .run(None, state, move |state| {
            state.dispatch_pending_commits();
            let (pointer_moved, topology_changed) = process_runtime_events(
                &runtime_event_rx,
                loop_state,
                layout_state,
                output_surfaces,
                &monotonic_clock,
            );
            if loop_state
                .removed_devices
                .contains(&devices.borrow().devices[&PRIMARY_DEVICE].devnum)
            {
                log::error!("Primary rendering GPU was removed; stopping the DRM session");
                loop_signal.stop();
                return;
            }
            if loop_state.take_topology_change(topology_changed)
                && reconcile_drm_outputs(
                    state,
                    output_surfaces,
                    devices,
                    &loop_handle,
                    runtime_event_tx,
                    loop_state,
                    layout_state,
                    &shared_input_dimensions,
                )
            {
                loop_state.mark_all_dirty();
                render_failures.retain(|scanout, _| {
                    output_surfaces
                        .iter()
                        .any(|entry| entry.scanout() == *scanout)
                });
            }
            process_frame_callback_requests(
                state,
                &loop_handle,
                loop_state,
                output_surfaces,
                start_time,
            );
            for entry in output_surfaces
                .iter()
                .filter(|entry| entry.enabled && entry.powered)
            {
                loop_state.presentation_scheduler.schedule_commit_timing(
                    entry.scanout(),
                    &loop_handle,
                    state,
                    &entry.output,
                );
            }
            state.dispatch_pending_commits();

            super::engine::event_loop_tick_and_request_render(state, ipc_server);
            if mem::take(&mut state.native.runtime.lid_policy_dirty) {
                state
                    .native
                    .queue_output_policy_projection(&state.wm.core.state.config.monitors);
            }
            if loop_state.session_active {
                let graphics = state.graphics.as_mut().expect("DRM graphics initialized");
                graphics.with_renderer(|renderer| {
                    process_output_configurations(
                        &mut state.native,
                        output_surfaces,
                        &mut devices.borrow_mut(),
                        renderer,
                        loop_state,
                    )
                });
            }
            let outputs_changed = state.native.project_completed_output_transactions();
            if outputs_changed {
                crate::monitor::refresh_monitor_layout(&mut state.ctx());
                state.native.push_command(
                    crate::backend::wayland::commands::WmCommand::SyncLayerExclusiveZones,
                );
                refresh_drm_layout_state(state, output_surfaces, layout_state);
                shared_input_dimensions.set(layout_state.layout);
                recover_pointer_after_output_change(state);
            }
            state.native.project_completed_output_power_requests();
            if loop_state.session_active {
                process_output_power_requests(&mut state.native, output_surfaces, loop_state);
            }
            state.native.project_completed_output_power_requests();
            if pointer_moved {
                loop_state.mark_pointer_output_dirty(
                    crate::types::Point::new(
                        state.native.runtime.pointer_location.x as i32,
                        state.native.runtime.pointer_location.y as i32,
                    ),
                    layout_state,
                );
            }
            super::engine::process_animations_and_request_render(state);
            process_commit_redraws(state, loop_state, output_surfaces);
            let bar_update_seq = state.wm.core.bar.update_seq();
            if loop_state.last_bar_update_seq != bar_update_seq {
                loop_state.last_bar_update_seq = bar_update_seq;
                loop_state.mark_all_dirty();
            }

            if state.wm.core.work.input_config {
                state.wm.core.work.input_config = false;
                crate::backend::wayland::input::drm::reconfigure_all_devices(
                    &mut state.native.runtime.tracked_devices,
                    &state.wm.core.state.config.input,
                );
            }

            if state.wm.core.work.cursor_config {
                state.wm.core.work.cursor_config = false;
                let cursor = &state.wm.core.state.config.cursor;
                cursor_manager.reload(&cursor.theme, cursor_size(cursor.size));
                state.native.cursor_config = cursor.clone();
                if let Some(xwm) = state.native.xwm.as_mut() {
                    cursor_manager.configure_xwayland(xwm);
                }
                loop_state.mark_all_dirty();
            }

            while let Ok(led_state) = led_state_rx.try_recv() {
                let leds = smithay::reexports::input::Led::from(led_state);
                for device in state.native.runtime.tracked_devices.iter_mut() {
                    use smithay::reexports::input::DeviceCapability;
                    if device.has_capability(DeviceCapability::Keyboard) {
                        device.led_update(leds);
                    }
                }
            }

            // Resolve cursor animation state. Its first dirty frame starts a
            // page-flip chain; subsequent vblanks advance it without an
            // independent compositor timer drifting against scanout.
            let animated = {
                let presentation = resolve_cursor(
                    &state.native.cursor_image_status,
                    state.native.cursor_icon_override,
                    state.native.runtime.dnd_icon.as_ref(),
                    state.native.runtime.cursor_hidden_by_touch,
                );
                resolved_cursor_icon(&presentation)
                    .is_some_and(|icon| cursor_manager.is_animated(icon, 1))
            };
            state.native.runtime.cursor_is_animated = animated;
            if animated {
                loop_state.mark_pointer_output_dirty(
                    crate::types::Point::new(
                        state.native.runtime.pointer_location.x as i32,
                        state.native.runtime.pointer_location.y as i32,
                    ),
                    layout_state,
                );
            }

            if let Some(keyboard_handle) = state.native.seat.get_keyboard() {
                process_cursor_warp(state, &pointer_handle, &keyboard_handle, loop_state);
            }

            render_outputs(
                state,
                output_surfaces,
                cursor_manager,
                &loop_handle,
                loop_state,
                render_failures,
                start_time,
            );

            // Retry persistent render failures at a bounded cadence. Dirty
            // outputs with a page flip in flight are woken by DRM vblank.
            render_retry_guard.ensure_armed(
                loop_state.has_renderable_dirty_outputs(),
                &loop_handle,
                |_| false,
            );

            state.dispatch_pending_commits();

            if state.native.display_handle.flush_clients().is_err() {
                loop_signal.stop();
            }
        })
        .expect("event loop run");
}

fn process_runtime_events(
    runtime_event_rx: &mpsc::Receiver<DrmRuntimeEvent>,
    loop_state: &mut DrmLoopState,
    layout_state: &DrmLayoutState,
    output_surfaces: &mut [OutputSurfaceEntry],
    monotonic_clock: &Clock<Monotonic>,
) -> (bool, bool) {
    let mut pointer_moved = false;
    let mut topology_changed = false;
    while let Ok(event) = runtime_event_rx.try_recv() {
        match event {
            DrmRuntimeEvent::SessionPaused => {
                loop_state.session_active = false;
            }
            DrmRuntimeEvent::SessionActivated => {
                // Keep in-flight flips intact: Smithay retains their buffers
                // across activate(), and only a real vblank may retire them.
                loop_state.session_active = true;
                loop_state.mark_all_dirty();
                // The dock may have disappeared while this session was inactive.
                topology_changed = true;
            }
            DrmRuntimeEvent::VBlank(crtc) => {
                if !loop_state.render_flags.contains_key(&crtc) {
                    continue;
                }
                loop_state
                    .presentation_scheduler
                    .presentation_completed(crtc, Instant::now());
                if let Some(entry) = output_surfaces
                    .iter_mut()
                    .find(|entry| entry.scanout() == crtc)
                {
                    let Some(surface) = entry.surface.as_mut() else {
                        loop_state.pending_crtcs.remove(&crtc);
                        continue;
                    };
                    match surface.frame_submitted() {
                        Ok(Some(mut metadata)) => {
                            let seq = loop_state.presentation_seq.entry(crtc).or_insert(0);
                            *seq += 1;
                            metadata.presentation_feedback.presented(
                                monotonic_clock.now(),
                                output_refresh(entry),
                                *seq,
                                wp_presentation_feedback::Kind::Vsync,
                            );
                        }
                        Ok(None) => {}
                        Err(err) => {
                            log::warn!("frame_submitted failed for {:?}: {err}", crtc);
                        }
                    }
                }
                loop_state.pending_crtcs.remove(&crtc);
            }
            DrmRuntimeEvent::PointerMoved { old_location } => {
                loop_state.mark_pointer_output_dirty(old_location, layout_state);
                pointer_moved = true;
            }
            DrmRuntimeEvent::OutputTopologyChanged => topology_changed = true,
            DrmRuntimeEvent::DeviceRemoved(devnum) => {
                loop_state.removed_devices.insert(devnum);
                topology_changed = true;
            }
        }
    }
    (pointer_moved, topology_changed)
}
