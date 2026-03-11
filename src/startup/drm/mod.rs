//! DRM/KMS bare-metal backend for running directly on hardware.

mod gpu;
mod input;
mod render;
mod state;

use std::collections::HashMap;
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::libinput::LibinputInputBackend;
use smithay::backend::libinput::LibinputSessionInterface;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportDma;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::Event as SessionEvent;
use smithay::backend::session::Session;
use smithay::reexports::calloop::{EventLoop, LoopSignal};
use smithay::reexports::drm::control::crtc;
use smithay::reexports::drm::control::Device as ControlDevice;
use smithay::reexports::input::Libinput;
use smithay::reexports::wayland_server::Display;
use smithay::utils::DeviceFd;

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::WaylandBackend;
use crate::backend::Backend as WmBackend;
use crate::startup::common_wayland::{
    init_wayland_globals, setup_wayland_socket, spawn_wayland_smoke_window, spawn_xwayland,
};
use crate::startup::wayland::cursor::CursorManager;
use crate::wm::Wm;

use self::gpu::build_output_surfaces;
use self::input::dispatch_libinput_event;
use self::render::render_drm_output;
use self::state::{
    sync_monitors_from_outputs_vec, OutputSurfaceEntry, SharedDrmState, CURSOR_SIZE,
    DEFAULT_SCREEN_HEIGHT, DEFAULT_SCREEN_WIDTH,
};
use super::autostart::run_autostart;
use crate::startup::wayland::input::apply_pending_warp;

// WARNING: This function is extremely fragile, do not refactor or mess with it without
// great care and patience for random ass segfaults. Yes, this is awful, leave it.
// Hours spent on this: ~3h
pub fn run() -> ! {
    log::info!("Starting DRM/KMS backend");

    let mut wm = Wm::new(WmBackend::Wayland(WaylandBackend::new()));
    init_wayland_globals(&mut wm);

    let mut event_loop: EventLoop<WaylandState> = EventLoop::try_new().expect("event loop");
    let loop_handle = event_loop.handle();

    let (mut session, notifier) = LibSeatSession::new().expect("libseat session");
    let seat_name = session.seat();
    log::info!("Session on seat: {seat_name}");

    let display: Display<WaylandState> = Display::new().expect("wayland display");
    let mut state = WaylandState::new(display, &loop_handle);
    state.attach_globals(&mut wm.g);
    if let WmBackend::Wayland(ref wayland) = wm.backend {
        wayland.attach_state(&mut state);
    }

    {
        let mut ctx = wm.ctx();
        crate::keyboard_layout::init_keyboard_layout(&mut ctx);
    }

    let (
        primary_gpu_path,
        mut drm_device,
        drm_notifier,
        drm_fd,
        gbm_device,
        egl_display,
        mut renderer,
    ) = init_gpu(&mut session, &seat_name);
    log::info!("Using GPU: {:?}", primary_gpu_path);

    state.attach_renderer(&mut renderer);
    state.init_dmabuf_global(
        renderer.dmabuf_formats().into_iter().collect(),
        Some(&egl_display),
    );
    state.init_screencopy_manager();

    let cursor_manager = init_cursor_manager(&mut renderer);

    let mut output_surfaces =
        build_output_surfaces(&mut drm_device, &mut renderer, &state, &gbm_device);
    for entry in &output_surfaces {
        state.space.map_output(&entry.output, (entry.x_offset, 0));
    }

    let (total_width, total_height) = compute_total_dimensions(&output_surfaces);

    sync_monitors_from_outputs_vec(&mut wm, &output_surfaces);
    {
        use crate::monitor::update_geom;
        update_geom(&mut wm.ctx());
    }

    let shared = init_shared_state(&output_surfaces, total_width, total_height);

    setup_wayland_socket(&loop_handle, &state);
    spawn_xwayland(&state, &loop_handle);
    wm.wayland_systray_runtime = crate::wayland_systray::WaylandSystrayRuntime::start();

    let mut libinput_context =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput_context
        .udev_assign_seat(&seat_name)
        .expect("libinput assign seat");
    libinput_context.dispatch().ok();

    let (libinput_tx, libinput_rx) = std::sync::mpsc::channel();
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());
    loop_handle
        .insert_source(libinput_backend, move |mut event, _, _state| {
            let _ = libinput_tx.send(event);
        })
        .expect("failed to insert libinput source");

    let keyboard_handle = state.keyboard.clone();
    let pointer_handle = state.pointer.clone();

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

    let mut ipc_server = crate::ipc::IpcServer::bind().ok();
    let start_time = std::time::Instant::now();
    let mut render_failures: HashMap<crtc::Handle, u32> = HashMap::new();

    if let Some(ref cmd) = wm.g.cfg.status_command {
        crate::bar::status::spawn_status_command(cmd);
    } else {
        crate::bar::status::spawn_default_status();
    }

    run_event_loop(
        event_loop,
        &mut wm,
        &mut state,
        &mut libinput_context,
        &shared,
        &mut output_surfaces,
        &mut renderer,
        &cursor_manager,
        &mut ipc_server,
        &mut render_failures,
        start_time,
        libinput_rx,
    );

    exit(0);
}

/// Initialize GPU, EGL, and renderer.
///
/// This function is safety-critical: it handles raw file descriptors and
/// unsafe EGL context creation. Do not reorder operations.
fn init_gpu(
    session: &mut LibSeatSession,
    seat_name: &str,
) -> (
    std::path::PathBuf,
    DrmDevice,
    smithay::backend::drm::DrmDeviceNotifier,
    DrmDeviceFd,
    GbmDevice<DrmDeviceFd>,
    EGLDisplay,
    GlesRenderer,
) {
    let (primary_gpu_path, mut drm_device, drm_notifier, drm_fd) =
        open_primary_gpu(session, seat_name);

    let gbm_device = GbmDevice::new(drm_fd.clone()).expect("GbmDevice::new");
    let egl_display = unsafe { EGLDisplay::new(gbm_device.clone()) }.expect("EGLDisplay::new");
    let egl_context = EGLContext::new(&egl_display).expect("EGLContext::new");
    let renderer = unsafe { GlesRenderer::new(egl_context) }.expect("GlesRenderer::new");

    (
        primary_gpu_path,
        drm_device,
        drm_notifier,
        drm_fd,
        gbm_device,
        egl_display,
        renderer,
    )
}

/// Initialize cursor manager from environment or defaults.
fn init_cursor_manager(renderer: &mut GlesRenderer) -> CursorManager {
    let cursor_theme = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".to_string());
    let cursor_size = std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(CURSOR_SIZE);
    CursorManager::new(renderer, &cursor_theme, cursor_size)
}

/// Compute total screen dimensions from output surfaces.
fn compute_total_dimensions(output_surfaces: &[OutputSurfaceEntry]) -> (i32, i32) {
    let total_width = output_surfaces
        .iter()
        .map(|s| s.x_offset + s.width)
        .max()
        .unwrap_or(DEFAULT_SCREEN_WIDTH);
    let total_height = output_surfaces
        .iter()
        .map(|s| s.height)
        .max()
        .unwrap_or(DEFAULT_SCREEN_HEIGHT);
    (total_width, total_height)
}

/// Initialize shared DRM state with render flags for each CRTC.
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
        .insert_source(drm_notifier, move |event, _metadata, _data| match event {
            DrmEvent::VBlank(crtc) => {
                let mut s = shared_vblank.lock().unwrap();
                if let Some(flag) = s.render_flags.get_mut(&crtc) {
                    *flag = true;
                }
                s.completed_crtcs.push(crtc);
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
    wm: &mut Wm,
    state: &mut WaylandState,
    libinput_context: &mut Libinput,
    shared: &Arc<Mutex<SharedDrmState>>,
    output_surfaces: &mut [OutputSurfaceEntry],
    renderer: &mut GlesRenderer,
    cursor_manager: &CursorManager,
    ipc_server: &mut Option<crate::ipc::IpcServer>,
    render_failures: &mut HashMap<crtc::Handle, u32>,
    start_time: std::time::Instant,
    libinput_rx: std::sync::mpsc::Receiver<
        smithay::backend::input::InputEvent<LibinputInputBackend>,
    >,
) {
    let loop_signal: LoopSignal = event_loop.get_signal();
    let keyboard_handle = state.keyboard.clone();
    let pointer_handle = state.pointer.clone();

    event_loop
        .run(Duration::from_millis(16), state, move |state| {
            state.attach_globals(&mut wm.g);

            process_completed_crtcs(state, shared, output_surfaces);

            process_libinput_events(
                libinput_context,
                state,
                wm,
                shared,
                &libinput_rx,
                &keyboard_handle,
                &pointer_handle,
            );

            arrange_layout(wm, state);

            process_ipc(ipc_server, wm, shared);

            process_animations(state, shared);

            process_cursor_warp(state, &pointer_handle, shared);

            render_outputs(
                wm,
                state,
                renderer,
                output_surfaces,
                cursor_manager,
                shared,
                render_failures,
                start_time,
            );

            if state.display_handle.flush_clients().is_err() {
                loop_signal.stop();
            }
        })
        .expect("event loop run");
}

/// Process frame submissions for completed CRTCs.
fn process_completed_crtcs(
    state: &mut WaylandState,
    shared: &Arc<Mutex<SharedDrmState>>,
    output_surfaces: &mut [OutputSurfaceEntry],
) {
    let completed_crtcs = {
        let mut s = shared.lock().unwrap();
        std::mem::take(&mut s.completed_crtcs)
    };
    for crtc in completed_crtcs {
        if let Some(entry) = output_surfaces.iter_mut().find(|entry| entry.crtc == crtc) {
            if let Err(err) = entry.surface.frame_submitted() {
                log::warn!("frame_submitted failed for {:?}: {err}", crtc);
            }
        }
    }
}

/// Process libinput events and dispatch to handlers.
fn process_libinput_events(
    libinput_context: &mut Libinput,
    state: &mut WaylandState,
    wm: &mut Wm,
    shared: &Arc<Mutex<SharedDrmState>>,
    libinput_rx: &std::sync::mpsc::Receiver<
        smithay::backend::input::InputEvent<LibinputInputBackend>,
    >,
    keyboard_handle: &smithay::input::keyboard::KeyboardHandle<WaylandState>,
    pointer_handle: &smithay::input::pointer::PointerHandle<WaylandState>,
) {
    if let Err(e) = libinput_context.dispatch() {
        log::error!("libinput dispatch error: {e}");
    }
    let mut any_input = false;
    while let Ok(event) = libinput_rx.try_recv() {
        if dispatch_libinput_event(event, state, wm, keyboard_handle, pointer_handle, shared) {
            any_input = true;
        }
    }
    if any_input {
        shared.lock().unwrap().mark_all_dirty();
    }
}

/// Arrange layout if clients exist and no animations are active.
fn arrange_layout(wm: &mut Wm, state: &mut WaylandState) {
    let mut ctx = wm.ctx();
    if !ctx.g.clients.is_empty() && !state.has_active_window_animations() {
        let selected_monitor_id = ctx.g.selected_monitor_id();
        crate::layouts::arrange(&mut ctx, Some(selected_monitor_id));
    }
}

/// Process IPC commands.
fn process_ipc(
    ipc_server: &mut Option<crate::ipc::IpcServer>,
    wm: &mut Wm,
    shared: &Arc<Mutex<SharedDrmState>>,
) {
    if let Some(server) = ipc_server.as_mut() {
        server.process_pending(wm);
        shared.lock().unwrap().mark_all_dirty();
    }
}

/// Process window animations.
fn process_animations(state: &mut WaylandState, shared: &Arc<Mutex<SharedDrmState>>) {
    state.sync_space_from_globals();
    state.tick_window_animations();
    if state.has_active_window_animations() {
        shared.lock().unwrap().mark_all_dirty();
    }
}

/// Apply compositor-side cursor warp.
fn process_cursor_warp(
    state: &mut WaylandState,
    pointer_handle: &smithay::input::pointer::PointerHandle<WaylandState>,
    shared: &Arc<Mutex<SharedDrmState>>,
) {
    let mut loc = shared.lock().unwrap().pointer_location;
    if apply_pending_warp(state, pointer_handle, &mut loc) {
        shared.lock().unwrap().pointer_location = loc;
        shared.lock().unwrap().mark_all_dirty();
    }
}

/// Render all outputs that need it.
#[allow(clippy::too_many_arguments)]
fn render_outputs(
    wm: &mut Wm,
    state: &mut WaylandState,
    renderer: &mut GlesRenderer,
    output_surfaces: &mut [OutputSurfaceEntry],
    cursor_manager: &CursorManager,
    shared: &Arc<Mutex<SharedDrmState>>,
    render_failures: &mut HashMap<crtc::Handle, u32>,
    start_time: std::time::Instant,
) {
    let (session_active, pointer_location, render_flags) = {
        let mut s = shared.lock().unwrap();
        let flags = s.render_flags.clone();
        for flag in s.render_flags.values_mut() {
            *flag = false;
        }
        (s.session_active, s.pointer_location, flags)
    };

    if session_active {
        for entry in output_surfaces.iter_mut() {
            let needs_render = render_flags.get(&entry.crtc).copied().unwrap_or(false);
            if !needs_render {
                continue;
            }
            let rendered = render_drm_output(
                wm,
                state,
                renderer,
                entry,
                cursor_manager,
                pointer_location,
                start_time,
            );

            if rendered {
                if let Some(failed_frames) = render_failures.remove(&entry.crtc) {
                    if failed_frames >= 3 {
                        log::info!(
                            "DRM render recovered on {:?} after {failed_frames} failed frames",
                            entry.crtc
                        );
                    }
                }
            } else {
                let failed_frames = render_failures.entry(entry.crtc).or_insert(0);
                *failed_frames += 1;

                if *failed_frames == 1 || *failed_frames % 60 == 0 {
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
}

fn open_primary_gpu(
    session: &mut LibSeatSession,
    seat_name: &str,
) -> (
    std::path::PathBuf,
    DrmDevice,
    smithay::backend::drm::DrmDeviceNotifier,
    DrmDeviceFd,
) {
    let gpus = smithay::backend::udev::all_gpus(seat_name).unwrap_or_default();
    let mut primary_gpu_path = None;
    let mut drm_device = None;
    let mut drm_notifier = None;
    let mut drm_fd = None;

    for gpu_path in gpus {
        if let Ok(fd) = session.open(
            &gpu_path,
            smithay::reexports::rustix::fs::OFlags::RDWR
                | smithay::reexports::rustix::fs::OFlags::CLOEXEC
                | smithay::reexports::rustix::fs::OFlags::NOCTTY
                | smithay::reexports::rustix::fs::OFlags::NONBLOCK,
        ) {
            let fd = DrmDeviceFd::new(DeviceFd::from(fd));
            if let Ok((device, notifier)) = DrmDevice::new(fd.clone(), true) {
                let has_connected = device
                    .resource_handles()
                    .map(|res| {
                        use smithay::reexports::drm::control::connector;
                        res.connectors().iter().any(|&c| {
                            device
                                .get_connector(c, false)
                                .map(|info| info.state() == connector::State::Connected)
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false);

                if has_connected || primary_gpu_path.is_none() {
                    primary_gpu_path = Some(gpu_path);
                    drm_device = Some(device);
                    drm_notifier = Some(notifier);
                    drm_fd = Some(fd);
                    if has_connected {
                        break;
                    }
                }
            }
        }
    }

    (
        primary_gpu_path.expect("no GPU found"),
        drm_device.expect("failed to open DRM device"),
        drm_notifier.expect("failed to create DRM notifier"),
        drm_fd.expect("failed to get DRM FD"),
    )
}
