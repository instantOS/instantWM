//! DRM/KMS bare-metal backend for running directly on hardware.

mod gpu;
mod input;
mod render;
mod state;

use std::collections::{HashMap, HashSet};
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent};
use smithay::backend::egl::{EGLContext, EGLDisplay};
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
use self::input::{dispatch_libinput_event, raw_event_to_input_event};
use self::render::render_drm_output;
use self::state::{
    sync_monitors_from_outputs_vec, OutputSurfaceEntry, SharedDrmState, CURSOR_SIZE,
    DEFAULT_SCREEN_HEIGHT, DEFAULT_SCREEN_WIDTH,
};
use super::autostart::run_autostart;
use crate::startup::wayland::input::apply_pending_warp;

type SharedDrm = Arc<Mutex<SharedDrmState>>;

pub fn run() -> ! {
    log::info!("Starting DRM/KMS backend");

    let mut event_loop: EventLoop<WaylandState> = EventLoop::try_new().expect("event loop");
    let loop_handle = event_loop.handle();

    let (mut wm, mut state) = initialize_wm_and_state(&loop_handle);

    let (mut session, notifier) = LibSeatSession::new().expect("libseat session");
    let seat_name = session.seat();
    log::info!("Session on seat: {seat_name}");

    init_keyboard_layout(&mut wm);

    let (primary_gpu_path, mut drm_device, drm_notifier, drm_fd) =
        open_primary_gpu(&mut session, &seat_name);
    log::info!("Using GPU: {:?}", primary_gpu_path);

    let (mut renderer, gbm_device, _egl_display) = initialize_renderer(&mut state, drm_fd);
    let cursor_manager = load_cursor_manager(&mut renderer);
    let (mut output_surfaces, shared) = initialize_outputs_and_shared(
        &mut wm,
        &mut state,
        &mut drm_device,
        &mut renderer,
        &gbm_device,
    );

    initialize_wayland_runtime(&loop_handle, &state, &mut wm);

    let mut libinput_context =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput_context
        .udev_assign_seat(&seat_name)
        .expect("libinput assign seat");
    libinput_context.dispatch().ok();

    let keyboard_handle = state.keyboard.clone();
    let pointer_handle = state.pointer.clone();

    let shared_session = Arc::clone(&shared);
    let mut session_drm_device = drm_device;
    let mut session_libinput = libinput_context.clone();
    loop_handle
        .insert_source(notifier, move |event, _, _data| match event {
            SessionEvent::PauseSession => {
                log::info!("Session paused (VT switch away) - suspending rendering");
                session_libinput.suspend();
                session_drm_device.pause();
                let mut s = shared_session.lock().unwrap();
                s.session_active = false;
                s.pending_crtcs.clear();
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

    let shared_vblank = Arc::clone(&shared);
    loop_handle
        .insert_source(drm_notifier, move |event, _metadata, _data| match event {
            DrmEvent::VBlank(crtc) => {
                let mut s = shared_vblank.lock().unwrap();
                if let Some(flag) = s.render_flags.get_mut(&crtc) {
                    *flag = true;
                }
                s.pending_crtcs.remove(&crtc);
                s.completed_crtcs.push(crtc);
            }
            DrmEvent::Error(err) => {
                log::error!("DRM error: {err}");
            }
        })
        .expect("drm notifier source");

    run_autostart();
    spawn_wayland_smoke_window();

    let mut ipc_server = crate::ipc::IpcServer::bind().ok();
    let start_time = std::time::Instant::now();
    let mut render_failures: HashMap<crtc::Handle, u32> = HashMap::new();

    let loop_signal: LoopSignal = event_loop.get_signal();
    event_loop
        .run(Duration::from_millis(16), &mut state, move |state| {
            tick(
                state,
                &mut wm,
                &shared,
                &mut output_surfaces,
                &mut libinput_context,
                &keyboard_handle,
                &pointer_handle,
                &mut ipc_server,
                &mut renderer,
                &cursor_manager,
                start_time,
                &mut render_failures,
            );

            if state.display_handle.flush_clients().is_err() {
                loop_signal.stop();
            }
        })
        .expect("event loop run");

    exit(0);
}

fn initialize_wm_and_state(
    loop_handle: &smithay::reexports::calloop::LoopHandle<'static, WaylandState>,
) -> (Wm, WaylandState) {
    let mut wm = Wm::new(WmBackend::Wayland(WaylandBackend::new()));
    init_wayland_globals(&mut wm);

    let display: Display<WaylandState> = Display::new().expect("wayland display");
    let mut state = WaylandState::new(display, loop_handle);
    state.attach_globals(&mut wm.g);
    if let WmBackend::Wayland(ref wayland) = wm.backend {
        wayland.attach_state(&mut state);
    }

    (wm, state)
}

fn init_keyboard_layout(wm: &mut Wm) {
    let mut ctx = wm.ctx();
    crate::keyboard_layout::init_keyboard_layout(&mut ctx);
}

fn initialize_renderer(
    state: &mut WaylandState,
    drm_fd: DrmDeviceFd,
) -> (GlesRenderer, GbmDevice<DrmDeviceFd>, EGLDisplay) {
    let gbm_device = GbmDevice::new(drm_fd).expect("GbmDevice::new");
    let egl_display = unsafe { EGLDisplay::new(gbm_device.clone()) }.expect("EGLDisplay::new");
    let egl_context = EGLContext::new(&egl_display).expect("EGLContext::new");
    let mut renderer = unsafe { GlesRenderer::new(egl_context) }.expect("GlesRenderer::new");

    state.attach_renderer(&mut renderer);
    state.init_dmabuf_global(
        renderer.dmabuf_formats().into_iter().collect(),
        Some(&egl_display),
    );
    state.init_screencopy_manager();

    (renderer, gbm_device, egl_display)
}

fn load_cursor_manager(renderer: &mut GlesRenderer) -> CursorManager {
    let cursor_theme = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".to_string());
    let cursor_size = std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(CURSOR_SIZE);
    CursorManager::new(renderer, &cursor_theme, cursor_size)
}

fn initialize_outputs_and_shared(
    wm: &mut Wm,
    state: &mut WaylandState,
    drm_device: &mut DrmDevice,
    renderer: &mut GlesRenderer,
    gbm_device: &GbmDevice<DrmDeviceFd>,
) -> (Vec<OutputSurfaceEntry>, SharedDrm) {
    let output_surfaces = build_output_surfaces(drm_device, renderer, state, gbm_device);
    for entry in &output_surfaces {
        state.space.map_output(&entry.output, (entry.x_offset, 0));
    }

    sync_monitors_from_outputs_vec(wm, &output_surfaces);
    crate::monitor::update_geom(&mut wm.ctx());

    let (total_width, total_height) = output_dimensions(&output_surfaces);
    let shared = Arc::new(Mutex::new(SharedDrmState::new(total_width, total_height)));
    {
        let mut s = shared.lock().unwrap();
        for entry in &output_surfaces {
            s.render_flags.insert(entry.crtc, true);
        }
    }

    (output_surfaces, shared)
}

fn output_dimensions(output_surfaces: &[OutputSurfaceEntry]) -> (i32, i32) {
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

fn initialize_wayland_runtime(
    loop_handle: &smithay::reexports::calloop::LoopHandle<'static, WaylandState>,
    state: &WaylandState,
    wm: &mut Wm,
) {
    setup_wayland_socket(loop_handle, state);
    spawn_xwayland(state, loop_handle);
    wm.wayland_systray_runtime = crate::wayland_systray::WaylandSystrayRuntime::start();
}

fn tick(
    state: &mut WaylandState,
    wm: &mut Wm,
    shared: &SharedDrm,
    output_surfaces: &mut [OutputSurfaceEntry],
    libinput_context: &mut Libinput,
    keyboard_handle: &smithay::input::keyboard::KeyboardHandle<WaylandState>,
    pointer_handle: &smithay::input::pointer::PointerHandle<WaylandState>,
    ipc_server: &mut Option<crate::ipc::IpcServer>,
    renderer: &mut GlesRenderer,
    cursor_manager: &CursorManager,
    start_time: std::time::Instant,
    render_failures: &mut HashMap<crtc::Handle, u32>,
) {
    state.attach_globals(&mut wm.g);

    process_completed_frames(shared, output_surfaces);
    process_libinput_events(
        state,
        wm,
        shared,
        libinput_context,
        keyboard_handle,
        pointer_handle,
    );
    update_scene_state(state, wm, shared, ipc_server);
    apply_pending_pointer_warp(state, shared, pointer_handle);

    let (session_active, pointer_location, render_flags, pending) = take_render_snapshot(shared);
    if !session_active {
        return;
    }

    for entry in output_surfaces.iter_mut() {
        let needs_render = render_flags.get(&entry.crtc).copied().unwrap_or(false);
        if !needs_render {
            continue;
        }
        // Never attempt to render on a CRTC whose previous page flip has
        // not completed yet.  Doing so would fail at queue_buffer (EBUSY)
        // and permanently leak the swapchain slot acquired by next_buffer.
        if pending.contains(&entry.crtc) {
            // Content is dirty but CRTC is busy — re-mark so we retry
            // once the VBlank arrives and clears the pending state.
            shared.lock().unwrap().render_flags.insert(entry.crtc, true);
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
            shared.lock().unwrap().pending_crtcs.insert(entry.crtc);
        }
        update_render_failures(shared, render_failures, entry.crtc, rendered);
    }
}

fn process_completed_frames(shared: &SharedDrm, output_surfaces: &mut [OutputSurfaceEntry]) {
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

fn process_libinput_events(
    state: &mut WaylandState,
    wm: &mut Wm,
    shared: &SharedDrm,
    libinput_context: &mut Libinput,
    keyboard_handle: &smithay::input::keyboard::KeyboardHandle<WaylandState>,
    pointer_handle: &smithay::input::pointer::PointerHandle<WaylandState>,
) {
    if let Err(e) = libinput_context.dispatch() {
        log::error!("libinput dispatch error: {e}");
    }

    let mut any_input = false;
    for raw_event in libinput_context.by_ref() {
        if let Some(event) = raw_event_to_input_event(raw_event) {
            if dispatch_libinput_event(event, state, wm, keyboard_handle, pointer_handle, shared) {
                any_input = true;
            }
        }
    }

    if any_input {
        mark_all_dirty(shared);
    }
}

fn update_scene_state(
    state: &mut WaylandState,
    wm: &mut Wm,
    shared: &SharedDrm,
    ipc_server: &mut Option<crate::ipc::IpcServer>,
) {
    {
        let mut ctx = wm.ctx();
        if !ctx.g.clients.is_empty() && !state.has_active_window_animations() {
            let selected_monitor_id = ctx.g.selected_monitor_id();
            crate::layouts::arrange(&mut ctx, Some(selected_monitor_id));
        }
    }

    if let Some(server) = ipc_server.as_mut() {
        server.process_pending(wm);
        mark_all_dirty(shared);
    }

    state.sync_space_from_globals();
    state.tick_window_animations();
    if state.has_active_window_animations() {
        mark_all_dirty(shared);
    }
}

fn apply_pending_pointer_warp(
    state: &mut WaylandState,
    shared: &SharedDrm,
    pointer_handle: &smithay::input::pointer::PointerHandle<WaylandState>,
) {
    let mut loc = shared.lock().unwrap().pointer_location;
    if apply_pending_warp(state, pointer_handle, &mut loc) {
        let mut s = shared.lock().unwrap();
        s.pointer_location = loc;
        s.mark_all_dirty();
    }
}

fn take_render_snapshot(
    shared: &SharedDrm,
) -> (
    bool,
    smithay::utils::Point<f64, smithay::utils::Logical>,
    HashMap<crtc::Handle, bool>,
    HashSet<crtc::Handle>,
) {
    let mut s = shared.lock().unwrap();
    let flags = s.render_flags.clone();
    let pending = s.pending_crtcs.clone();
    for flag in s.render_flags.values_mut() {
        *flag = false;
    }
    (s.session_active, s.pointer_location, flags, pending)
}

fn update_render_failures(
    shared: &SharedDrm,
    render_failures: &mut HashMap<crtc::Handle, u32>,
    crtc: crtc::Handle,
    rendered: bool,
) {
    if rendered {
        if let Some(failed_frames) = render_failures.remove(&crtc) {
            if failed_frames >= 3 {
                log::info!(
                    "DRM render recovered on {:?} after {failed_frames} failed frames",
                    crtc
                );
            }
        }
        return;
    }

    let failed_frames = render_failures.entry(crtc).or_insert(0);
    *failed_frames += 1;

    if *failed_frames == 1 || *failed_frames % 60 == 0 {
        log::warn!(
            "DRM render failed on {:?} (consecutive failures: {})",
            crtc,
            *failed_frames
        );
    }

    // If rendering fails before a successful submission, no vblank may arrive
    // for this CRTC. Re-mark it dirty so the main loop keeps retrying and
    // transient failures do not deadlock output updates.
    shared.lock().unwrap().render_flags.insert(crtc, true);
}

fn mark_all_dirty(shared: &SharedDrm) {
    shared.lock().unwrap().mark_all_dirty();
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
