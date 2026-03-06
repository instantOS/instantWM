//! DRM/KMS bare-metal backend for running directly on hardware.
//!
//! Uses libseat for session management, udev for GPU discovery, libinput
//! for input, and DRM/GBM/EGL for rendering.
//!
//! # Frame pacing
//!
//! Rendering is vblank-driven: each `DrmEvent::VBlank` signals that the
//! previous buffer has been scanned out and a new frame can be submitted.
//! A `needs_render` flag per output (keyed by CRTC handle) is set on every
//! VBlank and cleared once a frame has been queued.  Input events also set
//! the flag so the cursor moves without waiting for the next VBlank.
//!
//! # Input
//!
//! libinput is kept as a raw `Libinput` context (not registered as a calloop
//! source) and polled manually inside the main loop tick, exactly like the
//! winit backend calls `winit_loop.dispatch_new_events`.  This gives full
//! access to `wm` and `state` in the same closure, so the same generic input
//! handlers from `startup::wayland::input` can be called directly.
//!
//! Regular mice produce `InputEvent::PointerMotion` (relative deltas).
//! Tablets and touch screens produce `InputEvent::PointerMotionAbsolute`.
//! Both paths are handled.
//!
//! # Session management
//!
//! On VT switch away (`SessionEvent::PauseSession`) rendering is suspended.
//! On VT switch back (`SessionEvent::ActivateSession`) rendering resumes and
//! every output is marked dirty for a full repaint.

use std::collections::HashMap;
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent, GbmBufferedSurface};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::input::InputEvent;
use smithay::backend::libinput::{
    LibinputInputBackend, LibinputSessionInterface, PointerScrollAxis,
};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::texture::TextureRenderElement;

use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Bind, ImportDma};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev;
use smithay::desktop::space::render_output;
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::calloop::{EventLoop, LoopSignal};
use smithay::reexports::input::{event, event::EventTrait, Event as LibinputRawEvent, Libinput};
use smithay::reexports::wayland_server::Display;
use smithay::utils::{DeviceFd, Physical, Point, Rectangle};

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::WaylandBackend;
use crate::backend::Backend as WmBackend;

use crate::startup::common_wayland::{
    build_bar_elements, init_wayland_globals, send_frame_callbacks, setup_wayland_socket,
    spawn_wayland_smoke_window, spawn_xwayland,
};
use crate::startup::wayland::cursor::CursorManager;
use crate::startup::wayland::input::{
    handle_keyboard, handle_pointer_axis, handle_pointer_button, handle_pointer_motion_absolute,
    handle_pointer_motion_relative,
};
use crate::types::*;
use crate::wm::Wm;

use super::autostart::run_autostart;

use smithay::reexports::drm::control::{connector, crtc, Device as ControlDevice};

/// Default screen dimensions when no DRM outputs are detected.
const DEFAULT_SCREEN_WIDTH: i32 = 1280;
const DEFAULT_SCREEN_HEIGHT: i32 = 800;

/// Nominal cursor size in pixels to load from the xcursor theme.
const CURSOR_SIZE: u32 = 24;

// ---------------------------------------------------------------------------
// Render element enum — includes TextureRenderElement for the cursor sprite
// ---------------------------------------------------------------------------

render_elements! {
    pub DrmExtras<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Cursor=TextureRenderElement<GlesTexture>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Per-output runtime state
// ═══════════════════════════════════════════════════════════════════════════

struct OutputSurfaceEntry {
    crtc: crtc::Handle,
    surface: GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>,
    output: Output,
    damage_tracker: OutputDamageTracker,
    /// Logical x-offset of this output in the global compositor coordinate space.
    x_offset: i32,
    /// Logical pixel width of this output.
    width: i32,
    /// Logical pixel height of this output.
    height: i32,
}

// ═══════════════════════════════════════════════════════════════════════════
// Shared state between calloop callbacks and the main loop closure
// ═══════════════════════════════════════════════════════════════════════════

/// State that must be visible both inside calloop source callbacks (session
/// notifier, DRM notifier) **and** inside the main event-loop closure.
/// Wrapped in `Arc<Mutex<…>>` so it can be captured by multiple `move`
/// closures.
struct SharedDrmState {
    /// Whether the compositor currently owns the DRM device (i.e. we are on
    /// the active VT).  Set to `false` on `PauseSession`, `true` on
    /// `ActivateSession`.
    session_active: bool,
    /// Per-CRTC flag: `true` when a new frame should be rendered.  Set by
    /// VBlank events and by input / layout changes; cleared after a buffer is
    /// successfully queued.
    render_flags: HashMap<crtc::Handle, bool>,
    /// Current pointer position in logical compositor coordinates.
    pointer_location: Point<f64, smithay::utils::Logical>,
    /// Total compositor width (sum of all output widths) for pointer clamping.
    total_width: i32,
    /// Maximum output height for pointer clamping.
    total_height: i32,
    /// CRTCs that emitted a vblank and need `frame_submitted()` processing.
    completed_crtcs: Vec<crtc::Handle>,
}

impl SharedDrmState {
    fn new(total_width: i32, total_height: i32) -> Self {
        Self {
            session_active: true,
            render_flags: HashMap::new(),
            pointer_location: Point::from(((total_width / 2) as f64, (total_height / 2) as f64)),
            total_width,
            total_height,
            completed_crtcs: Vec::new(),
        }
    }

    /// Mark every output as needing a new frame.
    fn mark_all_dirty(&mut self) {
        for flag in self.render_flags.values_mut() {
            *flag = true;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Entry point
// ═══════════════════════════════════════════════════════════════════════════

pub fn run() -> ! {
    log::info!("Starting DRM/KMS backend");

    let mut wm = Wm::new(WmBackend::Wayland(WaylandBackend::new()));
    init_wayland_globals(&mut wm);

    let mut event_loop: EventLoop<WaylandState> = EventLoop::try_new().expect("event loop");
    let loop_handle = event_loop.handle();

    // ── Session ──────────────────────────────────────────────────────
    let (mut session, notifier) = LibSeatSession::new().expect("libseat session");
    let seat_name = session.seat();
    log::info!("Session on seat: {seat_name}");

    // ── Wayland display ──────────────────────────────────────────────
    let display: Display<WaylandState> = Display::new().expect("wayland display");
    let mut state = WaylandState::new(display, &loop_handle);
    state.attach_globals(&mut wm.g);
    if let WmBackend::Wayland(ref wayland) = wm.backend {
        wayland.attach_state(&mut state);
    }

    // Apply the initial keyboard layout if configured.
    {
        let mut ctx = wm.ctx();
        crate::keyboard_layout::init_keyboard_layout(&mut ctx);
    }

    // ── GPU discovery ────────────────────────────────────────────────
    let gpus = udev::all_gpus(&seat_name).unwrap_or_default();
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

    let primary_gpu_path = primary_gpu_path.expect("no GPU found");
    let mut drm_device = drm_device.expect("failed to open DRM device");
    let drm_notifier = drm_notifier.expect("failed to create DRM notifier");
    let drm_fd = drm_fd.expect("failed to get DRM FD");

    log::info!("Using GPU: {:?}", primary_gpu_path);

    // ── GBM + EGL + GLES renderer ────────────────────────────────────
    let gbm_device = GbmDevice::new(drm_fd.clone()).expect("GbmDevice::new");
    let egl_display = unsafe { EGLDisplay::new(gbm_device.clone()) }.expect("EGLDisplay::new");
    let egl_context = EGLContext::new(&egl_display).expect("EGLContext::new");
    let mut renderer = unsafe { GlesRenderer::new(egl_context) }.expect("GlesRenderer::new");

    state.attach_renderer(&mut renderer);
    state.init_dmabuf_global(renderer.dmabuf_formats().into_iter().collect());
    state.init_screencopy_manager();

    // ── Cursor textures ──────────────────────────────────────────────
    // Respect the standard XCURSOR_THEME / XCURSOR_SIZE environment variables.
    let cursor_theme = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".to_string());
    let cursor_size = std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(CURSOR_SIZE);
    let cursor_manager = CursorManager::new(&mut renderer, &cursor_theme, cursor_size);

    let gbm_allocator = GbmAllocator::new(
        gbm_device.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );

    // ── Scan connectors and create outputs ───────────────────────────
    let color_formats: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Xrgb8888];
    let renderer_formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();

    let mut output_surfaces: Vec<OutputSurfaceEntry> = Vec::new();
    let mut output_x_offset: i32 = 0;
    let mut _mon_idx_counter: usize = 0;

    {
        let res = drm_device.resource_handles().expect("drm resource_handles");
        let mut used_crtcs: Vec<crtc::Handle> = Vec::new();

        for &conn_handle in res.connectors() {
            let Ok(conn_info) = drm_device.get_connector(conn_handle, false) else {
                continue;
            };
            if conn_info.state() != connector::State::Connected
                && conn_info.state() != connector::State::Unknown
            {
                continue;
            }
            let modes = conn_info.modes();
            if modes.is_empty() {
                continue;
            }

            // Sort modes to find the best one: highest resolution (area), then highest refresh rate.
            let mut sorted_modes = modes.to_vec();
            sorted_modes.sort_by(|a, b| {
                let (aw, ah) = a.size();
                let (bw, bh) = b.size();
                (bw as u64 * bh as u64)
                    .cmp(&(aw as u64 * ah as u64))
                    .then_with(|| b.vrefresh().cmp(&a.vrefresh()))
            });

            // Always pick the largest resolution/highest refresh rate mode.
            let mode = sorted_modes[0];

            let encoder_crtcs: Vec<crtc::Handle> = conn_info
                .encoders()
                .iter()
                .filter_map(|&enc_h| drm_device.get_encoder(enc_h).ok())
                .flat_map(|enc| res.filter_crtcs(enc.possible_crtcs()))
                .collect();

            let Some(&picked_crtc) = encoder_crtcs.iter().find(|c| !used_crtcs.contains(c)) else {
                continue;
            };
            used_crtcs.push(picked_crtc);

            let drm_surface = drm_device
                .create_surface(picked_crtc, mode, &[conn_handle])
                .expect("create_surface");
            let gbm_surface = GbmBufferedSurface::new(
                drm_surface,
                gbm_allocator.clone(),
                color_formats,
                renderer_formats.iter().cloned(),
            )
            .expect("GbmBufferedSurface::new");

            let (mode_w, mode_h) = mode.size();
            let (mode_w, mode_h) = (mode_w as i32, mode_h as i32);
            let output_name = format!(
                "{}-{}",
                connector_type_name(conn_info.interface()),
                conn_info.interface_id()
            );
            log::info!(
                "Output {output_name}: {mode_w}x{mode_h}@{}Hz on CRTC {:?}",
                mode.vrefresh(),
                picked_crtc
            );

            let output = Output::new(
                output_name,
                PhysicalProperties {
                    size: {
                        let (mm_w, mm_h) = conn_info.size().unwrap_or((0, 0));
                        (mm_w as i32, mm_h as i32).into()
                    },
                    subpixel: Subpixel::Unknown,
                    make: "instantOS".into(),
                    model: "instantWM".into(),
                },
            );
            let out_mode = OutputMode {
                size: (mode_w, mode_h).into(),
                refresh: (mode.vrefresh() as i32) * 1000,
            };
            output.change_current_state(
                Some(out_mode),
                Some(smithay::utils::Transform::Normal),
                Some(Scale::Integer(1)),
                Some((output_x_offset, 0).into()),
            );
            output.set_preferred(out_mode);
            let _global = output.create_global::<WaylandState>(&state.display_handle);
            state.space.map_output(&output, (output_x_offset, 0));

            let damage_tracker = OutputDamageTracker::from_output(&output);

            output_surfaces.push(OutputSurfaceEntry {
                crtc: picked_crtc,
                surface: gbm_surface,
                output,
                damage_tracker,
                x_offset: output_x_offset,
                width: mode_w,
                height: mode_h,
            });
            output_x_offset += mode_w;
            _mon_idx_counter += 1;
        }
    }

    let total_width = output_x_offset.max(DEFAULT_SCREEN_WIDTH);
    let total_height = output_surfaces
        .iter()
        .map(|s| s.height)
        .max()
        .unwrap_or(DEFAULT_SCREEN_HEIGHT);

    // Sync instantWM monitor state from the detected outputs.
    sync_monitors_from_outputs_vec(&mut wm, &output_surfaces);

    // Ensure the generic monitor bookkeeping (bar position, work rects,
    // screen dimensions) is consistent, matching what the winit backend
    // does via `update_geom`.
    {
        use crate::monitor::update_geom;
        update_geom(&mut wm.ctx());
    }

    // ── Shared mutable DRM state ─────────────────────────────────────
    let shared = Arc::new(Mutex::new(SharedDrmState::new(total_width, total_height)));
    {
        let mut s = shared.lock().unwrap();
        for entry in &output_surfaces {
            s.render_flags.insert(entry.crtc, true);
        }
    }

    // ── Wayland socket + XWayland ────────────────────────────────────
    setup_wayland_socket(&loop_handle, &state);
    spawn_xwayland(&state, &loop_handle);
    wm.wayland_systray_runtime = crate::wayland_systray::WaylandSystrayRuntime::start();

    // ── libinput ─────────────────────────────────────────────────────
    // Keep the raw Libinput context and poll it manually in the main loop
    // (where both `wm` and `state` are in scope), rather than registering it
    // as a calloop source.  This is the same pattern the winit backend uses
    // with `winit_loop.dispatch_new_events`.
    let mut libinput_context =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput_context
        .udev_assign_seat(&seat_name)
        .expect("libinput assign seat");
    libinput_context.dispatch().ok();

    // Clone handles upfront so the main loop closure can pass them by ref.
    let keyboard_handle = state.keyboard.clone();
    let pointer_handle = state.pointer.clone();

    // ── Session events ───────────────────────────────────────────────
    let shared_session = Arc::clone(&shared);
    let mut session_drm_device = drm_device;
    let mut session_libinput = libinput_context.clone();
    loop_handle
        .insert_source(notifier, move |event, _, _data| match event {
            SessionEvent::PauseSession => {
                log::info!("Session paused (VT switch away) — suspending rendering");
                session_libinput.suspend();
                session_drm_device.pause();
                shared_session.lock().unwrap().session_active = false;
            }
            SessionEvent::ActivateSession => {
                log::info!("Session activated (VT switch back) — resuming rendering");
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

    // ── DRM vblank events ────────────────────────────────────────────
    // Each VBlank means the previously submitted buffer has been scanned out
    // and the swapchain slot is free.  Mark the corresponding output ready for
    // the next frame.
    let shared_vblank = Arc::clone(&shared);
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

    run_autostart();
    spawn_wayland_smoke_window();

    let mut ipc_server = crate::ipc::IpcServer::bind().ok();
    let start_time = std::time::Instant::now();

    let loop_signal: LoopSignal = event_loop.get_signal();

    // Use a 16 ms timeout (60 FPS) to avoid excessive CPU usage and potential
    // hangs while still ensuring timely frame and input processing.
    event_loop
        .run(Duration::from_millis(16), &mut state, move |state| {
            state.attach_globals(&mut wm.g);

            // ── Retire completed page-flips ───────────────────────────
            // Smithay's GBM surface requires `frame_submitted()` on vblank;
            // otherwise the swapchain keeps old buffers pending and eventually
            // stalls page-flips.
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

            // ── Poll libinput ─────────────────────────────────────────
            // Dispatch all pending input events before running layout/render.
            // We hold the raw Libinput context and map each raw event to the
            // smithay InputEvent<LibinputInputBackend> variant manually,
            // mirroring what LibinputInputBackend::process_events does
            // internally.
            if let Err(e) = libinput_context.dispatch() {
                log::error!("libinput dispatch error: {e}");
            }
            let mut any_input = false;
            for raw_event in libinput_context.by_ref() {
                log::trace!("libinput raw event: {:?}", raw_event);
                if let Some(event) = raw_event_to_input_event(raw_event) {
                    if dispatch_libinput_event(
                        event,
                        state,
                        &mut wm,
                        &keyboard_handle,
                        &pointer_handle,
                        &shared,
                    ) {
                        any_input = true;
                    }
                }
            }
            if any_input {
                shared.lock().unwrap().mark_all_dirty();
            }

            // ── Layout + IPC ─────────────────────────────────────────
            {
                let mut ctx = wm.ctx();
                if !ctx.g.clients.is_empty() && !state.has_active_window_animations() {
                    let selected_monitor_id = ctx.g.selected_monitor_id();
                    crate::layouts::arrange(&mut ctx, Some(selected_monitor_id));
                }
            }
            if let Some(server) = ipc_server.as_mut() {
                server.process_pending(&mut wm);
                shared.lock().unwrap().mark_all_dirty();
            }
            state.sync_space_from_globals();
            state.tick_window_animations();
            if state.has_active_window_animations() {
                shared.lock().unwrap().mark_all_dirty();
            }

            // ── Render all outputs that have a pending frame ──────────
            let (session_active, pointer_location, render_flags) = {
                let mut s = shared.lock().unwrap();
                let flags = s.render_flags.clone();
                // Clear flags upfront so that if an input event arrives *during*
                // rendering it will set the flag again for the next frame.
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

                    render_drm_output(
                        &mut wm,
                        state,
                        &mut renderer,
                        entry,
                        &cursor_manager,
                        pointer_location,
                        start_time,
                    );
                }
            }

            if state.display_handle.flush_clients().is_err() {
                loop_signal.stop();
            }
        })
        .expect("event loop run");

    exit(0);
}

// ═══════════════════════════════════════════════════════════════════════════
// Input dispatch
// ═══════════════════════════════════════════════════════════════════════════

/// Map a raw `libinput::Event` to the corresponding smithay
/// `InputEvent<LibinputInputBackend>`.
///
/// This mirrors the match block inside `LibinputInputBackend::process_events`.
/// We replicate it here so that we can poll the raw `Libinput` context
/// directly in the main loop without going through calloop.
fn raw_event_to_input_event(event: LibinputRawEvent) -> Option<InputEvent<LibinputInputBackend>> {
    use event::{keyboard::KeyboardEvent, pointer::PointerEvent, DeviceEvent};
    Some(match event {
        LibinputRawEvent::Keyboard(KeyboardEvent::Key(e)) => InputEvent::Keyboard { event: e },
        LibinputRawEvent::Pointer(PointerEvent::Motion(e)) => {
            InputEvent::PointerMotion { event: e }
        }
        LibinputRawEvent::Pointer(PointerEvent::MotionAbsolute(e)) => {
            InputEvent::PointerMotionAbsolute { event: e }
        }
        LibinputRawEvent::Pointer(PointerEvent::Button(e)) => {
            InputEvent::PointerButton { event: e }
        }
        LibinputRawEvent::Pointer(PointerEvent::ScrollWheel(e)) => InputEvent::PointerAxis {
            event: PointerScrollAxis::Wheel(e),
        },
        LibinputRawEvent::Pointer(PointerEvent::ScrollFinger(e)) => InputEvent::PointerAxis {
            event: PointerScrollAxis::Finger(e),
        },
        LibinputRawEvent::Pointer(PointerEvent::ScrollContinuous(e)) => InputEvent::PointerAxis {
            event: PointerScrollAxis::Continuous(e),
        },
        LibinputRawEvent::Device(DeviceEvent::Added(e)) => InputEvent::DeviceAdded {
            device: EventTrait::device(&e),
        },
        LibinputRawEvent::Device(DeviceEvent::Removed(e)) => InputEvent::DeviceRemoved {
            device: EventTrait::device(&e),
        },
        // Touch, gesture, tablet tool, switch events — not yet handled.
        _ => return None,
    })
}

/// Handle one libinput `InputEvent`, calling the appropriate generic handler
/// from `startup::wayland::input`.
///
/// Returns `true` if the event should trigger a repaint (almost all input
/// events do).
fn dispatch_libinput_event(
    event: InputEvent<LibinputInputBackend>,
    state: &mut WaylandState,
    wm: &mut Wm,
    keyboard_handle: &smithay::input::keyboard::KeyboardHandle<WaylandState>,
    pointer_handle: &smithay::input::pointer::PointerHandle<WaylandState>,
    shared: &Arc<Mutex<SharedDrmState>>,
) -> bool {
    let (total_w, total_h) = {
        let s = shared.lock().unwrap();
        (s.total_width, s.total_height)
    };

    match event {
        // ── Keyboard ─────────────────────────────────────────────────
        InputEvent::Keyboard { event } => {
            log::debug!(
                "DRM keyboard event: keycode={:?} state={:?}",
                smithay::backend::input::KeyboardKeyEvent::key_code(&event),
                smithay::backend::input::KeyboardKeyEvent::state(&event)
            );
            handle_keyboard::<LibinputInputBackend>(wm, state, keyboard_handle, event);
            true
        }

        // ── Relative pointer motion (regular mouse) ───────────────────
        InputEvent::PointerMotion { event } => {
            let mut loc = shared.lock().unwrap().pointer_location;
            handle_pointer_motion_relative::<LibinputInputBackend>(
                wm,
                state,
                pointer_handle,
                keyboard_handle,
                event,
                &mut loc,
                total_w,
                total_h,
            );
            shared.lock().unwrap().pointer_location = loc;
            true
        }

        // ── Absolute pointer motion (tablet / touchscreen) ────────────
        InputEvent::PointerMotionAbsolute { event } => {
            let mut loc = shared.lock().unwrap().pointer_location;
            handle_pointer_motion_absolute::<LibinputInputBackend>(
                wm,
                state,
                pointer_handle,
                keyboard_handle,
                event,
                &mut loc,
                total_w,
                total_h,
            );
            shared.lock().unwrap().pointer_location = loc;
            true
        }

        // ── Pointer button ────────────────────────────────────────────
        InputEvent::PointerButton { event } => {
            let loc = shared.lock().unwrap().pointer_location;
            handle_pointer_button::<LibinputInputBackend>(
                wm,
                state,
                pointer_handle,
                keyboard_handle,
                event,
                loc,
            );
            true
        }

        // ── Pointer axis (scroll wheel / touchpad) ────────────────────
        InputEvent::PointerAxis { event } => {
            let loc = shared.lock().unwrap().pointer_location;
            handle_pointer_axis::<LibinputInputBackend>(
                wm,
                state,
                pointer_handle,
                keyboard_handle,
                event,
                loc,
            );
            true
        }

        // All other event kinds (touch, gesture, tablet tool, switch, …)
        // are not handled by the WM yet.
        _ => false,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Rendering
// ═══════════════════════════════════════════════════════════════════════════

/// Render one frame for a single DRM output.
///
/// Returns `true` if a buffer was successfully queued for scanout (so the
/// caller can clear the `needs_render` flag and wait for the next VBlank),
/// or `false` on any error.
fn render_drm_output(
    wm: &mut Wm,
    state: &mut WaylandState,
    renderer: &mut GlesRenderer,
    entry: &mut OutputSurfaceEntry,
    cursor_manager: &CursorManager,
    pointer_location: Point<f64, smithay::utils::Logical>,
    start_time: std::time::Instant,
) -> bool {
    // Acquire the next buffer from the GBM swapchain.
    let (dmabuf, age) = match entry.surface.next_buffer() {
        Ok(buf) => buf,
        Err(e) => {
            log::trace!("next_buffer: {e}");
            return false;
        }
    };

    let mut dmabuf_clone = dmabuf.clone();
    let Ok(mut target) = renderer.bind(&mut dmabuf_clone) else {
        log::warn!("renderer bind failed");
        return false;
    };

    // ── Build render elements ─────────────────────────────────────────

    let mut custom_elements: Vec<DrmExtras> = Vec::new();

    // Bar
    for elem in build_bar_elements(wm, renderer) {
        custom_elements.push(DrmExtras::Memory(elem));
    }

    // Window borders
    for elem in crate::startup::wayland::render::wayland_border_elements_shared(&wm.g, state) {
        custom_elements.push(DrmExtras::Solid(elem));
    }

    // Cursor — rendered on top of everything.
    // The global pointer location is translated into per-output local
    // coordinates so that the cursor sits at the right position on each
    // output in a multi-monitor setup.
    let local_pointer = Point::from((
        pointer_location.x - entry.x_offset as f64,
        pointer_location.y,
    ));
    if let Some(cursor_elem) = cursor_manager.render_element(
        local_pointer,
        &state.cursor_image_status,
        state.cursor_icon_override,
    ) {
        custom_elements.push(DrmExtras::Cursor(cursor_elem));
    }

    // ── Render ───────────────────────────────────────────────────────

    let render_result = render_output(
        &entry.output,
        renderer,
        &mut target,
        1.0,
        age as usize,
        [&state.space],
        &custom_elements,
        &mut entry.damage_tracker,
        [0.05, 0.05, 0.07, 1.0],
    );

    // Screencopy (wlr-screencopy-v1)
    crate::backend::wayland::compositor::screencopy::submit_pending_screencopies(
        &mut state.pending_screencopies,
        renderer,
        &target,
        &entry.output,
        start_time,
    );
    drop(target);

    // ── Submit buffer ─────────────────────────────────────────────────

    match render_result {
        Ok(result) => {
            let damage: Option<Vec<Rectangle<i32, Physical>>> = result.damage.cloned();
            if let Err(e) = entry.surface.queue_buffer(None, damage, ()) {
                log::warn!("queue_buffer: {e}");
                return false;
            }
        }
        Err(e) => {
            log::warn!("render_output: {:?}", e);
            return false;
        }
    }

    // ── Frame callbacks ───────────────────────────────────────────────
    send_frame_callbacks(state, &entry.output, start_time.elapsed());

    true
}

// ═══════════════════════════════════════════════════════════════════════════
// Initialisation helpers
// ═══════════════════════════════════════════════════════════════════════════

fn sync_monitors_from_outputs_vec(wm: &mut Wm, surfaces: &[OutputSurfaceEntry]) {
    wm.g.monitors.clear();
    let tag_template = wm.g.cfg.tag_template.clone();

    for (i, surface) in surfaces.iter().enumerate() {
        let x = surface.x_offset;
        let y = 0i32;
        let w = surface.width;
        let h = surface.height;

        let mut mon = crate::types::Monitor::new_with_values(
            wm.g.cfg.mfact,
            wm.g.cfg.nmaster,
            wm.g.cfg.showbar,
            wm.g.cfg.topbar,
        );
        mon.num = i as i32;
        mon.monitor_rect = Rect { x, y, w, h };
        mon.work_rect = Rect { x, y, w, h };
        mon.current_tag = 1;
        mon.prev_tag = 1;
        mon.tag_set = [1, 1];
        mon.init_tags(&tag_template);
        mon.update_bar_position(wm.g.cfg.bar_height);
        wm.g.monitors.push(mon);
    }

    wm.g.cfg.screen_width = surfaces
        .iter()
        .map(|s| s.x_offset + s.width)
        .max()
        .unwrap_or(DEFAULT_SCREEN_WIDTH);
    wm.g.cfg.screen_height = surfaces
        .iter()
        .map(|s| s.height)
        .max()
        .unwrap_or(DEFAULT_SCREEN_HEIGHT);

    if wm.g.monitors.is_empty() {
        let mut mon = crate::types::Monitor::new_with_values(
            wm.g.cfg.mfact,
            wm.g.cfg.nmaster,
            wm.g.cfg.showbar,
            wm.g.cfg.topbar,
        );
        mon.monitor_rect = Rect {
            x: 0,
            y: 0,
            w: DEFAULT_SCREEN_WIDTH,
            h: DEFAULT_SCREEN_HEIGHT,
        };
        mon.work_rect = Rect {
            x: 0,
            y: 0,
            w: DEFAULT_SCREEN_WIDTH,
            h: DEFAULT_SCREEN_HEIGHT,
        };
        mon.init_tags(&tag_template);
        mon.update_bar_position(wm.g.cfg.bar_height);
        wm.g.monitors.push(mon);
    }

    for (i, mon) in wm.g.monitors.iter_mut() {
        mon.num = i as i32;
    }

    if wm.g.selected_monitor_id() >= wm.g.monitors.count() {
        wm.g.set_selected_monitor(0);
    }
}

fn connector_type_name(interface: connector::Interface) -> &'static str {
    match interface {
        connector::Interface::DVII => "DVI-I",
        connector::Interface::DVID => "DVI-D",
        connector::Interface::DVIA => "DVI-A",
        connector::Interface::SVideo => "S-Video",
        connector::Interface::DisplayPort => "DP",
        connector::Interface::HDMIA => "HDMI-A",
        connector::Interface::HDMIB => "HDMI-B",
        connector::Interface::EmbeddedDisplayPort => "eDP",
        connector::Interface::VGA => "VGA",
        connector::Interface::LVDS => "LVDS",
        connector::Interface::DSI => "DSI",
        connector::Interface::DPI => "DPI",
        connector::Interface::Composite => "Composite",
        _ => "Unknown",
    }
}
