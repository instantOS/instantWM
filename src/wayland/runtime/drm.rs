//! DRM/KMS bare-metal backend for running directly on hardware.

use std::collections::HashMap;
use std::process::exit;
use std::sync::{Arc, Mutex};

use smithay::backend::drm::{DrmDevice, DrmEvent};
use smithay::backend::libinput::LibinputInputBackend;
use smithay::backend::libinput::LibinputSessionInterface;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::session::Event as SessionEvent;
use smithay::backend::session::Session;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::reexports::calloop::{EventLoop, LoopSignal};
use smithay::reexports::drm::control::crtc;
use smithay::reexports::input::Libinput;
use smithay::reexports::wayland_server::Display;

use crate::backend::Backend as WmBackend;
use crate::backend::wayland::WaylandBackend;
use crate::backend::wayland::compositor::WaylandState;
use crate::config::config_toml::CursorConfig;
use crate::startup::autostart::run_autostart;
use crate::wayland::common::{
    ensure_dbus_session, init_wayland_globals, setup_wayland_socket, spawn_wayland_smoke_window,
    spawn_xwayland,
};
use crate::wayland::init::drm::init_gpu;
use crate::wayland::input::apply_pending_warp;
use crate::wayland::render::drm::{
    CursorManager, OutputSurfaceEntry, SharedDrmState, build_output_surfaces, render_drm_output,
};
use crate::wm::Wm;

// WARNING: This function is extremely fragile, do not refactor or mess with it without
// great care and patience for random ass segfaults. Yes, this is awful, leave it.
// Hours spent on this: ~3h
pub fn run() -> ! {
    log::info!("Starting DRM/KMS backend");
    ensure_dbus_session();

    let mut wm = Box::new(Wm::new(WmBackend::new_wayland(WaylandBackend::new())));
    if let Some(wayland) = wm.backend.wayland_data_mut() {
        init_wayland_globals(&mut wm.g, wayland);
    }

    let event_loop: EventLoop<WaylandState> = EventLoop::try_new().expect("event loop");
    let loop_handle = event_loop.handle();

    let (mut session, notifier) = LibSeatSession::new().expect("libseat session");
    let seat_name = session.seat();
    log::info!("Session on seat: {seat_name}");

    let display: Display<WaylandState> = Display::new().expect("wayland display");

    let (
        primary_gpu_path,
        mut drm_device,
        drm_notifier,
        _drm_fd,
        gbm_device,
        egl_display,
        renderer,
    ) = init_gpu(&mut session, &seat_name);
    log::info!("Using GPU: {:?}", primary_gpu_path);

    let dmabuf_formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();
    let mut state = WaylandState::new(display, &loop_handle, *wm, Some(renderer));

    let mut ctx = state.wm.ctx();
    crate::keyboard_layout::init_keyboard_layout(&mut ctx);

    state.init_dmabuf_global(dmabuf_formats, Some(&egl_display));

    state.with_renderer(|state, renderer| {
        state.bind_egl_to_display(renderer);
    });

    state.init_screencopy_manager();

    let cursor_manager = init_cursor_manager(&state.cursor_config);

    let mut output_surfaces = state.with_renderer(|state, renderer| {
        build_output_surfaces(&mut drm_device, renderer, state, &gbm_device)
    });
    for entry in &output_surfaces {
        state.space.map_output(&entry.output, (entry.x_offset, 0));
    }

    let (total_width, total_height) = compute_total_dimensions(&output_surfaces);

    crate::wayland::render::drm::sync_monitors_from_outputs_vec(&mut state.wm.g, &output_surfaces);
    {
        use crate::monitor::update_geom;
        update_geom(&mut state.wm.ctx());
    }

    let shared = init_shared_state(&output_surfaces, total_width, total_height);

    setup_wayland_socket(&loop_handle, &state);
    spawn_xwayland(&state, &loop_handle);

    // Initialize Wayland systray runtime - only applicable for Wayland backend
    if let WmBackend::Wayland(data) = &mut state.wm.backend {
        data.wayland_systray_runtime = crate::systray::wayland::WaylandSystrayRuntime::start();
    }

    let mut libinput_context =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput_context
        .udev_assign_seat(&seat_name)
        .expect("libinput assign seat");

    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());
    loop_handle
        .insert_source(libinput_backend, move |event, _, state| {
            state.pending_libinput_events.push(event);
        })
        .expect("failed to insert libinput source");

    setup_session_handlers(
        &loop_handle,
        notifier,
        &shared,
        &mut libinput_context,
        drm_device,
    );

    setup_drm_vblank_handler(&loop_handle, drm_notifier, &shared);

    run_autostart();
    spawn_wayland_smoke_window();

    let ipc_server = crate::ipc::IpcServer::bind().ok();
    let start_time = std::time::Instant::now();
    let mut render_failures: HashMap<crtc::Handle, u32> = HashMap::new();

    let core = state.wm.ctx().core();
    crate::runtime::spawn_status_bar(&core);

    let (led_state_tx, led_state_rx) = std::sync::mpsc::channel();
    state.led_state_tx = Some(led_state_tx);

    run_event_loop(
        event_loop,
        &mut state,
        &shared,
        &mut output_surfaces,
        &cursor_manager,
        ipc_server,
        &mut render_failures,
        start_time,
        led_state_rx,
    );

    exit(0);
}

/// Initialize cursor manager from environment or defaults.
fn init_cursor_manager(config: &CursorConfig) -> CursorManager {
    let cursor_theme = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| config.theme.clone());
    let cursor_size = std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(config.size);
    CursorManager::new(&cursor_theme, cursor_size as u8)
}

/// Compute total screen dimensions from output surfaces.
fn compute_total_dimensions(output_surfaces: &[OutputSurfaceEntry]) -> (i32, i32) {
    let total_width = output_surfaces
        .iter()
        .map(|s| s.x_offset + s.width)
        .max()
        .unwrap_or(crate::wayland::render::drm::DEFAULT_SCREEN_WIDTH);
    let total_height = output_surfaces
        .iter()
        .map(|s| s.height)
        .max()
        .unwrap_or(crate::wayland::render::drm::DEFAULT_SCREEN_HEIGHT);
    (total_width, total_height)
}

/// Initialize shared DRM state with render flags for each CRTC.
///
/// The `Arc<Mutex<SharedDrmState>>` is necessary because multiple execution contexts
/// access the shared state concurrently:
/// - Session handlers (pause/activate) run in **libseat callback context** (VT switch events)
/// - Vblank handlers run in **DRM event callback context** (page flip completion)
/// - The event loop processes state in **normal thread context**
///
/// These contexts all access `render_flags`, `completed_crtcs`, and `pending_crtcs`,
/// so a mutex is required to synchronize access across callback threads.
fn init_shared_state(
    output_surfaces: &[OutputSurfaceEntry],
    total_width: i32,
    total_height: i32,
) -> Arc<Mutex<SharedDrmState>> {
    let shared = Arc::new(Mutex::new(SharedDrmState::new(total_width, total_height)));
    {
        let mut s = shared.lock().unwrap();
        for entry in output_surfaces {
            s.render_flags.insert(entry.crtc, true);
            s.output_hit_regions
                .push(crate::wayland::render::drm::OutputHitRegion {
                    crtc: entry.crtc,
                    x_offset: entry.x_offset,
                    width: entry.width,
                });
            if entry.vrr_active {
                s.vrr_crtcs.insert(entry.crtc);
            }
        }
    }
    shared
}

/// Setup session pause/activate handlers for VT switching.
fn setup_session_handlers(
    loop_handle: &calloop::LoopHandle<WaylandState>,
    notifier: smithay::backend::session::libseat::LibSeatSessionNotifier,
    shared: &Arc<Mutex<SharedDrmState>>,
    libinput_context: &mut Libinput,
    drm_device: DrmDevice,
) {
    let shared_session = Arc::clone(shared);
    let mut session_libinput = libinput_context.clone();
    let mut session_drm_device = drm_device;

    loop_handle
        .insert_source(notifier, move |event, _, _data| match event {
            SessionEvent::PauseSession => {
                log::info!("Session paused (VT switch away) - suspending rendering");
                session_libinput.suspend();
                session_drm_device.pause();
                shared_session.lock().unwrap().session_active = false;
            }
            SessionEvent::ActivateSession => {
                log::info!("Session activated (VT switch back) - resuming rendering");
                if let Err(err) = session_libinput.resume() {
                    log::error!("failed to resume libinput context: {:?}", err);
                }
                if let Err(err) = session_drm_device.activate(false) {
                    log::error!("failed to reactivate DRM device: {err}");
                }
                let mut s = shared_session.lock().unwrap();
                s.session_active = true;
                s.mark_all_dirty();
            }
        })
        .expect("session source");
}

/// Setup DRM vblank handler for render synchronization.
fn setup_drm_vblank_handler(
    loop_handle: &calloop::LoopHandle<WaylandState>,
    drm_notifier: smithay::backend::drm::DrmDeviceNotifier,
    shared: &Arc<Mutex<SharedDrmState>>,
) {
    let shared_vblank = Arc::clone(shared);
    loop_handle
        .insert_source(drm_notifier, move |event, meta, _data| match event {
            DrmEvent::VBlank(crtc) => {
                // Extract presentation time from VBlank metadata if available
                let presentation_time = meta
                    .and_then(|m| match m.time {
                        smithay::backend::drm::DrmEventTime::Monotonic(time) => Some(time),
                        smithay::backend::drm::DrmEventTime::Realtime(_) => None,
                    })
                    .unwrap_or(std::time::Duration::ZERO);

                let mut s = shared_vblank.lock().unwrap();
                // For fixed-rate displays, VBlank drives the render loop but
                // we only re-mark dirty when content has actually changed.
                // For VRR outputs, only content changes (input, animations,
                // surface commits) should trigger renders — not VBlank.
                if !s.vrr_crtcs.contains(&crtc)
                    && s.content_dirty
                    && let Some(flag) = s.render_flags.get_mut(&crtc)
                {
                    *flag = true;
                }
                s.completed_crtcs.push(crtc);

                // Store presentation time for frame clock updates
                if !presentation_time.is_zero() {
                    s.presentation_times.insert(crtc, presentation_time);
                }
            }
            DrmEvent::Error(err) => {
                log::error!("DRM error: {err}");
            }
        })
        .expect("drm notifier source");
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
    shared: &Arc<Mutex<SharedDrmState>>,
    output_surfaces: &mut [OutputSurfaceEntry],
    cursor_manager: &CursorManager,
    ipc_server: Option<crate::ipc::IpcServer>,
    render_failures: &mut HashMap<crtc::Handle, u32>,
    start_time: std::time::Instant,
    led_state_rx: std::sync::mpsc::Receiver<smithay::input::keyboard::LedState>,
) {
    let loop_signal: LoopSignal = event_loop.get_signal();
    let loop_handle = event_loop.handle();
    let pointer_handle = state.pointer.clone();

    // Register IPC server as a calloop source if available
    // This makes IPC event-driven rather than polled every iteration
    if let Some(ipc) = ipc_server {
        let shared_ipc = Arc::clone(shared);
        super::calloop_helpers::setup_ipc_source(loop_handle.clone(), ipc, move |ipc, state| {
            if ipc.process_pending(&mut state.wm) {
                state.wm.g.dirty.layout = true;
                let mut ctx = state.wm.ctx();
                crate::runtime::apply_monitor_config_if_dirty(&mut ctx);
                state.wm.g.dirty.space = true;
                shared_ipc.lock().unwrap().mark_all_dirty();
            }
        });
    }

    // Animation timer - fires every 16ms when animations are active.
    // The tick callback also handles LED state checks from the keyboard.
    // When animations are active, marks all DRM outputs dirty for re-render.
    let shared_anim = Arc::clone(shared);
    super::calloop_helpers::setup_animation_timer(
        loop_handle.clone(),
        move |state| {
            // Check LED state updates
            while let Ok(led_state) = led_state_rx.try_recv() {
                let leds = smithay::reexports::input::Led::from(led_state);
                for device in state.tracked_devices.iter_mut() {
                    use smithay::reexports::input::DeviceCapability;
                    if device.has_capability(DeviceCapability::Keyboard) {
                        device.led_update(leds);
                    }
                }
            }
            state.tick_window_animations();
        },
        move |state| {
            let active = state.has_active_window_animations();
            if active {
                shared_anim.lock().unwrap().mark_all_dirty();
            }
            active
        },
    );

    // Main event loop - no timeout needed since all work is event-driven
    // The timeout is only for safety in case we miss an event wakeup
    event_loop
        .run(None, state, move |state| {
            process_completed_crtcs(state, shared, output_surfaces);

            process_pending_libinput_events(state, shared);

            state.popups.cleanup();
            state.refresh_popup_grab();

            super::common::arrange_layout_if_dirty(state);

            if state.wm.g.dirty.input_config {
                state.wm.g.dirty.input_config = false;
                crate::wayland::input::drm::reconfigure_all_devices(
                    &mut state.tracked_devices,
                    &state.wm.g.cfg.input,
                );
            }

            // Drain and execute commands BEFORE sync/render so that
            // map/unmap from show_hide_wayland takes effect in the Space
            // before we build render elements.  Without this, invisible
            // windows would render for one extra frame after a tag switch.
            super::common::drain_and_execute_ops(state);

            if super::common::sync_space_if_dirty(state) {
                shared.lock().unwrap().mark_content_dirty();
            }

            process_cursor_warp(state, &pointer_handle, shared);

            // Surface commits from client windows set content_dirty_pending.
            // Propagate it to SharedDrmState so the VBlank handler will mark
            // render_flags.  This must be checked on every tick (not just inside
            // render_outputs) so that commits on an idle desktop wake up
            // rendering on the next VBlank even when no CRTC is currently dirty.
            if state.content_dirty_pending {
                state.content_dirty_pending = false;
                shared.lock().unwrap().mark_content_dirty();
            }

            state.with_renderer(|state, renderer| {
                render_outputs(
                    state,
                    renderer,
                    output_surfaces,
                    cursor_manager,
                    shared,
                    render_failures,
                    start_time,
                );
            });

            // Second drain pass for any commands queued during execute_command
            // or render (e.g. screencopy, surface lifecycle callbacks).
            super::common::drain_and_execute_ops(state);

            // Sync caches
            if let crate::backend::Backend::Wayland(data) = &state.wm.backend {
                data.backend.sync_cache(state);
            }

            if state.display_handle.flush_clients().is_err() {
                loop_signal.stop();
            }
        })
        .expect("event loop run");
}

/// Drain and dispatch queued libinput events.
///
/// Events are pushed into `WaylandState::pending_libinput_events` by the
/// calloop source callback (which doesn't have access to `Wm`).  We process
/// them here in the main event-loop body where `&mut Wm` is available.
fn process_pending_libinput_events(state: &mut WaylandState, shared: &Arc<Mutex<SharedDrmState>>) {
    let events: Vec<_> = std::mem::take(&mut state.pending_libinput_events);
    if events.is_empty() {
        return;
    }

    let (total_w, total_h) = {
        let s = shared.lock().unwrap();
        (s.total_width, s.total_height)
    };

    let mut any_input = false;
    for event in events {
        any_input |=
            crate::wayland::input::drm::dispatch_libinput_event(event, state, total_w, total_h);
    }

    if any_input {
        shared
            .lock()
            .unwrap()
            .mark_pointer_output_dirty(state.pointer_location.x as i32);
    }
}

/// Process frame submissions for completed CRTCs.
fn process_completed_crtcs(
    _state: &mut WaylandState,
    shared: &Arc<Mutex<SharedDrmState>>,
    output_surfaces: &mut [OutputSurfaceEntry],
) {
    let (completed_crtcs, presentation_times) = {
        let mut s = shared.lock().unwrap();
        let crtcs = std::mem::take(&mut s.completed_crtcs);
        let times = std::mem::take(&mut s.presentation_times);
        (crtcs, times)
    };
    if completed_crtcs.is_empty() {
        return;
    }
    for crtc in &completed_crtcs {
        if let Some(entry) = output_surfaces.iter_mut().find(|entry| entry.crtc == *crtc) {
            if let Err(err) = entry.surface.frame_submitted() {
                log::warn!("frame_submitted failed for {:?}: {err}", crtc);
            }
            // Update frame clock with presentation time if available
            if let Some(presentation_time) = presentation_times.get(crtc) {
                entry.frame_clock.presented(*presentation_time);
            }
        }
    }
    // Clear in-flight tracking so these CRTCs can render again.
    let mut s = shared.lock().unwrap();
    for crtc in &completed_crtcs {
        s.pending_crtcs.remove(crtc);
    }
}

/// Apply compositor-side cursor warp.
fn process_cursor_warp(
    state: &mut WaylandState,
    pointer_handle: &smithay::input::pointer::PointerHandle<WaylandState>,
    shared: &Arc<Mutex<SharedDrmState>>,
) {
    if apply_pending_warp(state, pointer_handle) {
        let mut s = shared.lock().unwrap();
        s.mark_all_dirty();
    }
}

/// Render all outputs that need it.
#[allow(clippy::too_many_arguments)]
fn render_outputs(
    state: &mut WaylandState,
    renderer: &mut GlesRenderer,
    output_surfaces: &mut [OutputSurfaceEntry],
    cursor_manager: &CursorManager,
    shared: &Arc<Mutex<SharedDrmState>>,
    render_failures: &mut HashMap<crtc::Handle, u32>,
    start_time: std::time::Instant,
) {
    let (session_active, render_flags, pending_crtcs) = {
        let mut s = shared.lock().unwrap();
        let flags = s.render_flags.clone();
        for flag in s.render_flags.values_mut() {
            *flag = false;
        }
        (s.session_active, flags, s.pending_crtcs.clone())
    };

    if !render_flags.values().any(|&v| v) {
        return;
    }

    let pointer_location = state.pointer_location;
    let mut any_rendered = false;

    if session_active {
        for entry in output_surfaces.iter_mut() {
            let needs_render = render_flags.get(&entry.crtc).copied().unwrap_or(false);
            if !needs_render {
                continue;
            }
            // Don't render if a page flip is already in flight — queue_buffer
            // would fail with EBUSY and leak a swapchain slot.
            if pending_crtcs.contains(&entry.crtc) {
                // Re-mark as dirty so we render after the VBlank arrives.
                shared.lock().unwrap().render_flags.insert(entry.crtc, true);
                continue;
            }

            let render_start = std::time::Instant::now();
            let rendered = render_drm_output(
                state,
                renderer,
                entry,
                cursor_manager,
                pointer_location,
                start_time,
            );
            let render_duration = render_start.elapsed();

            if rendered {
                any_rendered = true;
                // Update estimated render duration (exponential moving average)
                entry.last_render_duration = std::time::Duration::from_nanos(
                    (entry.last_render_duration.as_nanos() as f64 * 0.8
                        + render_duration.as_nanos() as f64 * 0.2) as u64,
                );

                shared.lock().unwrap().pending_crtcs.insert(entry.crtc);
                if let Some(failed_frames) = render_failures.remove(&entry.crtc)
                    && failed_frames >= 3
                {
                    log::info!(
                        "DRM render recovered on {:?} after {failed_frames} failed frames",
                        entry.crtc
                    );
                }
            } else {
                let failed_frames = render_failures.entry(entry.crtc).or_insert(0);
                *failed_frames += 1;

                if *failed_frames == 1 || (*failed_frames).is_multiple_of(60) {
                    log::warn!(
                        "DRM render failed on {:?} (consecutive failures: {})",
                        entry.crtc,
                        *failed_frames
                    );
                }

                shared.lock().unwrap().render_flags.insert(entry.crtc, true);
            }
        }
    }

    // If at least one frame was rendered, clear content_dirty so non-VRR
    // outputs skip rendering on subsequent VBlanks until something changes.
    if any_rendered {
        shared.lock().unwrap().clear_content_dirty();
    }
}
