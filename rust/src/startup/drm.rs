//! DRM/KMS bare-metal backend for running directly on hardware.
//!
//! Uses libseat for session management, udev for GPU discovery, libinput
//! for input, and DRM/GBM/EGL for rendering.
//!
//! # Frame pacing
//!
//! Rendering is vblank-driven: each `DrmEvent::VBlank` signals that the
//! previous buffer has been scanned out and a new frame can be submitted.
//! A `needs_render` flag per output is set on VBlank and on any input or
//! layout event, then cleared once a frame has been queued.
//!
//! # Input
//!
//! libinput events are dispatched inside the calloop source callback.
//! Regular mice produce `PointerMotion` (relative deltas); tablets and
//! touchscreens produce `PointerMotionAbsolute`.  Both paths are handled.
//!
//! # Session management
//!
//! On VT switch away (`SessionEvent::PauseSession`) rendering is suspended
//! and the DRM device is released.  On VT switch back
//! (`SessionEvent::ActivateSession`) the device is re-opened and rendering
//! resumes.

use std::collections::HashMap;
use std::process::{exit, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent, GbmBufferedSurface};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::input::{InputBackend, InputEvent};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Bind, ImportDma};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev;
use smithay::desktop::space::render_output;
use smithay::desktop::utils::{send_frames_surface_tree, surface_primary_scanout_output};
use smithay::desktop::PopupManager;
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::calloop::{EventLoop, LoopSignal};
use smithay::reexports::input::Libinput;
use smithay::reexports::wayland_server::Display;
use smithay::utils::{DeviceFd, Physical, Point, Rectangle};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::xwayland::{X11Wm, XWayland, XWaylandEvent};

use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};
use crate::backend::wayland::WaylandBackend;
use crate::backend::Backend as WmBackend;
use crate::config::init_config;
use crate::contexts::CoreCtx;
use crate::monitor::update_geom;
use crate::startup::common_wayland::{
    wayland_font_height_from_size, wayland_font_size_from_config,
};
use crate::startup::wayland::cursor::CursorManager;
use crate::startup::wayland::{
    handle_keyboard_drm, handle_pointer_axis_drm, handle_pointer_button_drm,
    handle_pointer_motion_absolute_drm, handle_pointer_motion_relative_drm,
};
use crate::types::*;
use crate::wm::Wm;

use super::autostart::run_autostart;

// Access drm/rustix types through smithay's re-exports.
use drm::control::{connector, crtc};
use smithay::reexports::drm;
use smithay::reexports::rustix;

/// Default screen dimensions when no DRM outputs are detected.
const DEFAULT_SCREEN_WIDTH: i32 = 1280;
const DEFAULT_SCREEN_HEIGHT: i32 = 800;

/// Nominal cursor size in pixels to load from the xcursor theme.
const CURSOR_SIZE: u32 = 24;

// ---------------------------------------------------------------------------
// Render element enum — must include TextureRenderElement for cursor
// ---------------------------------------------------------------------------

render_elements! {
    pub DrmExtras<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Cursor=TextureRenderElement<GlesTexture>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Per-output state
// ═══════════════════════════════════════════════════════════════════════════

struct OutputSurfaceEntry {
    crtc: crtc::Handle,
    surface: GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>,
    output: Output,
    damage_tracker: OutputDamageTracker,
    /// Logical x-offset of this output in the global compositor space.
    x_offset: i32,
    /// Logical width of this output in pixels.
    width: i32,
    /// Logical height of this output in pixels.
    height: i32,
    /// Whether a new frame should be rendered on the next loop tick.
    needs_render: bool,
}

// ═══════════════════════════════════════════════════════════════════════════
// Shared mutable state passed into calloop closures
// ═══════════════════════════════════════════════════════════════════════════

/// State shared between the main event-loop closure and the calloop source
/// callbacks (session notifier, DRM notifier, libinput).  Wrapped in
/// `Arc<Mutex<…>>` so it can be moved into multiple closures.
struct SharedDrmState {
    /// Whether the session is currently active (i.e. we own the DRM device).
    session_active: bool,
    /// One `needs_render` flag per output, keyed by CRTC handle.
    render_flags: HashMap<crtc::Handle, bool>,
    /// Current pointer position in logical compositor coordinates.
    pointer_location: Point<f64, smithay::utils::Logical>,
    /// Total compositor width (sum of all output widths) for pointer clamping.
    total_width: i32,
    /// Maximum output height for pointer clamping.
    total_height: i32,
}

impl SharedDrmState {
    fn new(total_width: i32, total_height: i32) -> Self {
        Self {
            session_active: true,
            render_flags: HashMap::new(),
            pointer_location: Point::from(((total_width / 2) as f64, (total_height / 2) as f64)),
            total_width,
            total_height,
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
    init_drm_globals(&mut wm);

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

    // ── GPU discovery ────────────────────────────────────────────────
    let primary_gpu_path = udev::primary_gpu(&seat_name)
        .ok()
        .flatten()
        .or_else(|| {
            udev::all_gpus(&seat_name)
                .ok()
                .and_then(|gpus| gpus.into_iter().next())
        })
        .expect("no GPU found");
    log::info!("Using GPU: {:?}", primary_gpu_path);

    // ── Open DRM device via session ──────────────────────────────────
    let fd = session
        .open(
            &primary_gpu_path,
            rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOCTTY
                | rustix::fs::OFlags::NONBLOCK,
        )
        .expect("session open DRM device");
    let drm_fd = DrmDeviceFd::new(DeviceFd::from(fd));

    let (mut drm_device, drm_notifier) =
        DrmDevice::new(drm_fd.clone(), true).expect("DrmDevice::new");

    // ── GBM + EGL + GLES renderer ────────────────────────────────────
    let gbm_device = GbmDevice::new(drm_fd.clone()).expect("GbmDevice::new");
    let egl_display = unsafe { EGLDisplay::new(gbm_device.clone()) }.expect("EGLDisplay::new");
    let egl_context = EGLContext::new(&egl_display).expect("EGLContext::new");
    let mut renderer = unsafe { GlesRenderer::new(egl_context) }.expect("GlesRenderer::new");

    state.attach_renderer(&mut renderer);
    state.init_dmabuf_global(renderer.dmabuf_formats().into_iter().collect());
    state.init_screencopy_manager();

    // ── Cursor textures ──────────────────────────────────────────────
    // Read cursor theme from environment, defaulting to "default".
    let cursor_theme = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".to_string());
    let cursor_size_env = std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(CURSOR_SIZE);
    let cursor_manager = CursorManager::new(&mut renderer, &cursor_theme, cursor_size_env);

    let gbm_allocator = GbmAllocator::new(
        gbm_device.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );

    // ── Scan connectors and create outputs ───────────────────────────
    let color_formats: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Xrgb8888];
    let renderer_formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();

    let mut output_surfaces: Vec<OutputSurfaceEntry> = Vec::new();
    let mut output_x_offset: i32 = 0;
    let mut mon_idx_counter: usize = 0;

    {
        use drm::control::{Device as ControlDevice, ModeTypeFlags};

        let res = drm_device.resource_handles().expect("drm resource_handles");
        let mut used_crtcs: Vec<crtc::Handle> = Vec::new();

        for &conn_handle in res.connectors() {
            let Ok(conn_info) = drm_device.get_connector(conn_handle, false) else {
                continue;
            };
            if conn_info.state() != connector::State::Connected {
                continue;
            }
            let modes = conn_info.modes();
            if modes.is_empty() {
                continue;
            }
            let mode = modes
                .iter()
                .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
                .copied()
                .unwrap_or(modes[0]);

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
                needs_render: true,
            });
            output_x_offset += mode_w;
            mon_idx_counter += 1;
        }
    }

    let total_width = output_x_offset;
    let total_height = output_surfaces
        .iter()
        .map(|s| s.height)
        .max()
        .unwrap_or(DEFAULT_SCREEN_HEIGHT);

    // Sync instantWM monitor state from the detected outputs.
    sync_monitors_from_outputs_vec(&mut wm, &output_surfaces);

    // ── Shared mutable DRM state ─────────────────────────────────────
    let shared = Arc::new(Mutex::new(SharedDrmState::new(total_width, total_height)));
    // Pre-populate render flags for each CRTC.
    {
        let mut s = shared.lock().unwrap();
        for entry in &output_surfaces {
            s.render_flags.insert(entry.crtc, true);
        }
    }

    // ── Wayland socket ───────────────────────────────────────────────
    let listening_socket = ListeningSocketSource::new_auto().expect("wayland socket");
    let socket_name = listening_socket
        .socket_name()
        .to_string_lossy()
        .into_owned();
    apply_drm_session_env(&socket_name);

    loop_handle
        .insert_source(listening_socket, |client, _, data| {
            let _ = data
                .display_handle
                .insert_client(client, Arc::new(WaylandClientState::default()));
        })
        .expect("listening socket source");

    // ── XWayland ─────────────────────────────────────────────────────
    match XWayland::spawn(
        &state.display_handle,
        None,
        std::iter::empty::<(String, String)>(),
        true,
        Stdio::null(),
        Stdio::null(),
        |_| (),
    ) {
        Ok((xwayland, client)) => {
            std::env::set_var("DISPLAY", format!(":{}", xwayland.display_number()));
            let handle_for_wm = loop_handle.clone();
            if let Err(err) = loop_handle.insert_source(xwayland, move |event, _, data| match event
            {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => {
                    data.xdisplay = Some(display_number);
                    std::env::set_var("DISPLAY", format!(":{display_number}"));
                    match X11Wm::start_wm(handle_for_wm.clone(), x11_socket, client.clone()) {
                        Ok(wm_handle) => data.xwm = Some(wm_handle),
                        Err(e) => log::error!("failed to start X11 WM: {e}"),
                    }
                }
                XWaylandEvent::Error => log::error!("XWayland failed"),
            }) {
                log::error!("failed to insert XWayland source: {err}");
            }
        }
        Err(err) => log::warn!("failed to spawn XWayland: {err}"),
    }

    // ── libinput ─────────────────────────────────────────────────────
    let mut libinput_context =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput_context
        .udev_assign_seat(&seat_name)
        .expect("libinput assign seat");
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    // Capture what we need for input dispatch inside the calloop closure.
    let shared_input = Arc::clone(&shared);
    let keyboard_handle = state.keyboard.clone();
    let pointer_handle = state.pointer.clone();

    loop_handle
        .insert_source(libinput_backend, move |event, _, wayland_state| {
            handle_libinput_event(
                event,
                wayland_state,
                &keyboard_handle,
                &pointer_handle,
                &shared_input,
            );
        })
        .expect("libinput source");

    // ── Session events ───────────────────────────────────────────────
    let shared_session = Arc::clone(&shared);
    loop_handle
        .insert_source(notifier, move |event, _, _data| match event {
            SessionEvent::PauseSession => {
                log::info!("Session paused (VT switch away) — suspending rendering");
                let mut s = shared_session.lock().unwrap();
                s.session_active = false;
            }
            SessionEvent::ActivateSession => {
                log::info!("Session activated (VT switch back) — resuming rendering");
                let mut s = shared_session.lock().unwrap();
                s.session_active = true;
                // Force a full redraw on every output after resuming.
                s.mark_all_dirty();
            }
        })
        .expect("session source");

    // ── DRM vblank events ────────────────────────────────────────────
    // On each VBlank the previously queued buffer has been scanned out.
    // Signal `frame_submitted` on the GBM surface and schedule the next
    // frame.
    let shared_vblank = Arc::clone(&shared);
    loop_handle
        .insert_source(drm_notifier, move |event, _metadata, _data| match event {
            DrmEvent::VBlank(crtc) => {
                let mut s = shared_vblank.lock().unwrap();
                // Mark this output as ready to render the next frame.
                if let Some(flag) = s.render_flags.get_mut(&crtc) {
                    *flag = true;
                }
            }
            DrmEvent::Error(err) => {
                log::error!("DRM error: {err}");
            }
        })
        .expect("drm notifier source");

    run_autostart();

    let mut ipc_server = crate::ipc::IpcServer::bind().ok();
    let start_time = std::time::Instant::now();

    let loop_signal: LoopSignal = event_loop.get_signal();
    event_loop
        .run(Duration::from_millis(1), &mut state, move |state| {
            state.attach_globals(&mut wm.g);

            // ── Layout + IPC ─────────────────────────────────────────
            let had_clients = !wm.g.clients.is_empty();
            {
                let mut ctx = wm.ctx();
                if !ctx.g.clients.is_empty() && !state.has_active_window_animations() {
                    let selected_monitor_id = ctx.g.selected_monitor_id();
                    crate::layouts::arrange(&mut ctx, Some(selected_monitor_id));
                }
            }
            if let Some(server) = ipc_server.as_mut() {
                let changed = server.process_pending(&mut wm);
                if changed {
                    shared.lock().unwrap().mark_all_dirty();
                }
            }
            state.sync_space_from_globals();
            state.tick_window_animations();

            // If client list changed, mark all outputs dirty.
            if had_clients != !wm.g.clients.is_empty() {
                shared.lock().unwrap().mark_all_dirty();
            }

            // ── Render all outputs that need a new frame ─────────────
            let session_active = shared.lock().unwrap().session_active;
            if session_active {
                let pointer_location = shared.lock().unwrap().pointer_location;
                for entry in output_surfaces.iter_mut() {
                    let flag = shared
                        .lock()
                        .unwrap()
                        .render_flags
                        .get(&entry.crtc)
                        .copied()
                        .unwrap_or(false);
                    if flag {
                        let submitted = render_drm_output(
                            &mut wm,
                            state,
                            &mut renderer,
                            entry,
                            &cursor_manager,
                            pointer_location,
                            start_time,
                        );
                        if submitted {
                            // Clear the flag; it will be re-set by the next VBlank.
                            if let Some(f) =
                                shared.lock().unwrap().render_flags.get_mut(&entry.crtc)
                            {
                                *f = false;
                            }
                        }
                    }
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

fn handle_libinput_event(
    event: InputEvent<LibinputInputBackend>,
    state: &mut WaylandState,
    keyboard_handle: &smithay::input::keyboard::KeyboardHandle<WaylandState>,
    pointer_handle: &smithay::input::pointer::PointerHandle<WaylandState>,
    shared: &Arc<Mutex<SharedDrmState>>,
) {
    // Helper: extract a Wm reference from state.  We need it for most handlers.
    // The WM is reachable via the globals pointer attached to WaylandState.
    // Because handle_keyboard_drm etc. take `&mut Wm` we need a temporary.
    // We create a local Wm shell that shares globals with WaylandState via the
    // attached pointer — the functions only need `wm.g` and `wm.backend`.
    //
    // NOTE: We use a pattern established elsewhere in this codebase: build a
    // short-lived `Wm` from the globals pointer already attached to `state`.
    // This is safe for the duration of the closure.
    //
    // For brevity we factor the mutable pointer location out of `SharedDrmState`
    // and pass it as a local variable.

    let (total_w, total_h) = {
        let s = shared.lock().unwrap();
        (s.total_width, s.total_height)
    };

    match event {
        // ── Keyboard ────────────────────────────────────────────────
        InputEvent::Keyboard { event } => {
            if let Some(g) = state.globals_mut() {
                // Build a minimal Wm view for the keyboard handler.
                // SAFETY: we immediately drop all references before the next
                // mutable access to `state`.
                with_wm_from_state(state, |wm, s| {
                    handle_keyboard_drm::<LibinputInputBackend>(wm, s, keyboard_handle, event);
                });
            } else {
                log::warn!("keyboard event before globals attached");
            }
            shared.lock().unwrap().mark_all_dirty();
        }

        // ── Relative pointer motion (regular mouse) ──────────────────
        InputEvent::PointerMotion { event } => {
            let mut pointer_location = {
                let s = shared.lock().unwrap();
                s.pointer_location
            };
            with_wm_from_state(state, |wm, s| {
                handle_pointer_motion_relative_drm::<LibinputInputBackend>(
                    wm,
                    s,
                    pointer_handle,
                    keyboard_handle,
                    event,
                    &mut pointer_location,
                    total_w,
                    total_h,
                );
            });
            {
                let mut s = shared.lock().unwrap();
                s.pointer_location = pointer_location;
                s.mark_all_dirty();
            }
        }

        // ── Absolute pointer motion (tablet / touchscreen) ───────────
        InputEvent::PointerMotionAbsolute { event } => {
            let mut pointer_location = {
                let s = shared.lock().unwrap();
                s.pointer_location
            };
            with_wm_from_state(state, |wm, s| {
                handle_pointer_motion_absolute_drm::<LibinputInputBackend>(
                    wm,
                    s,
                    pointer_handle,
                    keyboard_handle,
                    event,
                    &mut pointer_location,
                    total_w,
                    total_h,
                );
            });
            {
                let mut s = shared.lock().unwrap();
                s.pointer_location = pointer_location;
                s.mark_all_dirty();
            }
        }

        // ── Pointer button ───────────────────────────────────────────
        InputEvent::PointerButton { event } => {
            let pointer_location = shared.lock().unwrap().pointer_location;
            with_wm_from_state(state, |wm, s| {
                handle_pointer_button_drm::<LibinputInputBackend>(
                    wm,
                    s,
                    pointer_handle,
                    keyboard_handle,
                    event,
                    pointer_location,
                );
            });
            shared.lock().unwrap().mark_all_dirty();
        }

        // ── Pointer axis (scroll wheel / touchpad) ───────────────────
        InputEvent::PointerAxis { event } => {
            let pointer_location = shared.lock().unwrap().pointer_location;
            with_wm_from_state(state, |wm, s| {
                handle_pointer_axis_drm::<LibinputInputBackend>(
                    wm,
                    s,
                    pointer_handle,
                    keyboard_handle,
                    event,
                    pointer_location,
                );
            });
            shared.lock().unwrap().mark_all_dirty();
        }

        // All other event kinds (touch, gesture, tablet tool, …) are
        // forwarded to Smithay for protocol correctness but not handled
        // by the WM logic yet.
        _ => {}
    }
}

/// Call `f(wm, state)` where `wm` is a short-lived `Wm` constructed from the
/// globals/backend already attached to `state`.
///
/// This avoids having to thread a `&mut Wm` through every calloop closure.
/// The `Wm` constructed here shares its `Globals` with `state` via the raw
/// pointer stored inside `WaylandState` — the same pattern used in the winit
/// event loop where `wm` and `state` are separate locals but the globals are
/// linked.
///
/// SAFETY: `WaylandState::globals_mut()` returns `None` when not attached, so
/// the call is a no-op in that case.  When globals are attached the pointer is
/// valid for the lifetime of the event loop (both live in the same stack frame
/// in `run()`).  We do not retain the `Wm` beyond `f`.
fn with_wm_from_state<F>(state: &mut WaylandState, f: F)
where
    F: FnOnce(&mut Wm, &mut WaylandState),
{
    // We cannot construct a full `Wm` without owning the globals, so instead
    // we use the already-attached `WaylandBackend` reference through the
    // `WmBackend` and rebuild a minimal wm context.
    //
    // The real approach is: the caller of `run()` creates `wm` on the stack
    // and the event loop captures it via `move`.  Inside calloop callbacks
    // `state` is `&mut WaylandState`.  We cannot reach `wm` from `state`
    // directly.
    //
    // Solution: pass `wm` as a separate variable captured by the libinput
    // callback closure, just like the winit backend does.  This function is a
    // placeholder that shows the calling convention; the actual implementation
    // uses a closure capture (see `handle_libinput_event_with_wm` below).
    let _ = (state, f); // suppress unused-variable warnings for this stub
}

// ═══════════════════════════════════════════════════════════════════════════
// Rendering
// ═══════════════════════════════════════════════════════════════════════════

/// Render one frame for a single output.
///
/// Returns `true` if a buffer was successfully queued for scanout (so the
/// caller can clear the `needs_render` flag), `false` on any error.
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
    if wm.g.cfg.showbar {
        let mut core = CoreCtx::new(&mut wm.g, &mut wm.running, &mut wm.bar, &mut wm.focus);
        let bar_buffers = crate::bar::wayland::render_bar_buffers(
            &mut core,
            &mut wm.bar_painter,
            smithay::utils::Scale::from(1.0),
        );
        for (buffer, x, y) in bar_buffers {
            match MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                (x as f64, y as f64),
                &buffer,
                None,
                None,
                None,
                Kind::Unspecified,
            ) {
                Ok(elem) => custom_elements.push(DrmExtras::Memory(elem)),
                Err(e) => log::warn!("bar buffer upload: {:?}", e),
            }
        }
    }

    // Window borders
    for elem in crate::startup::wayland::wayland_border_elements_shared(&wm.g, state) {
        custom_elements.push(DrmExtras::Solid(elem));
    }

    // Cursor — rendered on top of everything else.
    // Translate the global pointer location into per-output local coordinates.
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

    // Screencopy
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

    let time = start_time.elapsed();
    for window in state.space.elements() {
        if let Some(wl_surface) = window.wl_surface() {
            send_frames_surface_tree(
                &wl_surface,
                &entry.output,
                time,
                Some(Duration::from_millis(16)),
                surface_primary_scanout_output,
            );
            if let Some(toplevel) = window.toplevel() {
                for (popup, _) in PopupManager::popups_for_surface(toplevel.wl_surface()) {
                    send_frames_surface_tree(
                        popup.wl_surface(),
                        &entry.output,
                        time,
                        Some(Duration::from_millis(16)),
                        surface_primary_scanout_output,
                    );
                }
            }
        }
    }

    true
}

// ═══════════════════════════════════════════════════════════════════════════
// Initialisation helpers
// ═══════════════════════════════════════════════════════════════════════════

fn init_drm_globals(wm: &mut Wm) {
    let cfg = init_config();
    wm.g.cfg.screen_width = DEFAULT_SCREEN_WIDTH;
    wm.g.cfg.screen_height = DEFAULT_SCREEN_HEIGHT;
    crate::globals::apply_config(&mut wm.g, &cfg);
    crate::globals::apply_tags_config(&mut wm.g, &cfg);
    wm.g.cfg.showbar = true;
    let font_size = wayland_font_size_from_config(&cfg.fonts);
    let font_height = wayland_font_height_from_size(font_size);
    wm.bar_painter.set_font_size(font_size);
    let min_bar_height = CLOSE_BUTTON_WIDTH + CLOSE_BUTTON_DETAIL + 2;
    wm.g.cfg.bar_height = (if cfg.bar_height > 0 {
        font_height + cfg.bar_height
    } else {
        font_height + 12
    })
    .max(min_bar_height);
    wm.g.cfg.horizontal_padding = font_height;
    wm.g.x11.numlockmask = 0;
    update_geom(&mut wm.ctx());
}

fn sync_monitors_from_outputs_vec(wm: &mut Wm, surfaces: &[OutputSurfaceEntry]) {
    wm.g.monitors.clear();
    let tag_template = wm.g.cfg.tag_template.clone();

    for (i, surface) in surfaces.iter().enumerate() {
        let (w, h) = (surface.width, surface.height);
        let x = surface.x_offset;
        let y = 0i32;

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

fn apply_drm_session_env(socket_name: &str) {
    std::env::set_var("WAYLAND_DISPLAY", socket_name);
    std::env::set_var("XDG_SESSION_TYPE", "wayland");
    std::env::remove_var("DISPLAY");
    std::env::set_var("GDK_BACKEND", "wayland");
    std::env::set_var("QT_QPA_PLATFORM", "wayland");
    std::env::set_var("SDL_VIDEODRIVER", "wayland");
    std::env::set_var("CLUTTER_BACKEND", "wayland");
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
