//! Shared Wayland runtime setup and per-tick logic for all backends.
//!
//! Bootstrap uses [`create_wayland_wm_boxed`] and [`new_wayland_event_loop_and_state`], then
//! [`attach_wayland_backend_state`], [`attach_gles_renderer_and_protocols`], and the socket /
//! autostart helpers. DRM inserts session/GPU/libinput between socket setup and autostart.
//!
//! Per-tick logic: [`event_loop_tick`], [`process_window_animations`].

use smithay::backend::egl::EGLDisplay;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::reexports::calloop::{EventLoop, LoopHandle};
use smithay::reexports::wayland_server::Display;

use crate::backend::Backend as WmBackend;
use crate::backend::wayland::WaylandBackend;
use crate::backend::wayland::compositor::WaylandState;
use crate::wm::Wm;

/// D-Bus session, boxed [`Wm`] with Wayland backend, and [`crate::wayland::common::init_wayland_globals`].
pub(crate) fn create_wayland_wm_boxed() -> Box<Wm> {
    crate::wayland::common::ensure_dbus_session();
    let mut wm = Box::new(Wm::new(WmBackend::new_wayland(WaylandBackend::new())));
    if let Some(wayland) = wm.backend.wayland_data_mut() {
        crate::wayland::common::init_wayland_globals(&mut wm.g, wayland);
    }
    wm
}

/// Calloop [`EventLoop`], Wayland [`Display`], and [`WaylandState`].
pub(crate) fn new_wayland_event_loop_and_state() -> (EventLoop<'static, WaylandState>, WaylandState)
{
    let event_loop = EventLoop::try_new().expect("wayland event loop");
    let loop_handle = event_loop.handle();
    let display = Display::new().expect("wayland display");
    let state = WaylandState::new(display, &loop_handle);
    (event_loop, state)
}

/// Attach GLES renderer, dmabuf global, and screencopy protocol (winit and DRM).
///
/// Pass `egl_display` when it comes from elsewhere (e.g. DRM `init_gpu`). Pass [`None`]
/// for winit so the display is read from `renderer` after [`WaylandState::attach_renderer`]
/// (avoids overlapping borrows from the winit backend).
pub fn attach_gles_renderer_and_protocols(
    state: &mut WaylandState,
    renderer: &mut GlesRenderer,
    egl_display: Option<&EGLDisplay>,
) {
    state.attach_renderer(renderer);
    let egl_for_dmabuf = egl_display.or_else(|| Some(renderer.egl_context().display()));
    state.init_dmabuf_global(
        renderer.dmabuf_formats().into_iter().collect(),
        egl_for_dmabuf,
    );
    state.init_screencopy_manager();
}

/// Wire the Smithay compositor state into [`WaylandBackend`].
pub fn attach_wayland_backend_state(wm: &mut Box<Wm>, state: &mut WaylandState) {
    if let WmBackend::Wayland(data) = &mut wm.backend {
        data.backend.attach_state(state);
    }
}

/// Listening socket, XWayland spawn, and StatusNotifier systray thread — shared by both runtimes.
pub fn setup_wayland_listen_socket_xwayland_systray(
    loop_handle: &LoopHandle<'static, WaylandState>,
    state: &WaylandState,
    wm: &mut Box<Wm>,
) {
    let _socket_name = crate::wayland::common::setup_wayland_socket(loop_handle, state);
    crate::wayland::common::spawn_xwayland(state, loop_handle);
    if let WmBackend::Wayland(data) = &mut wm.backend {
        data.wayland_systray_runtime = crate::systray::wayland::WaylandSystrayRuntime::start();
    }
}

/// Startup commands, smoke window, IPC listener registration, and status-bar ping source.
pub fn wayland_autostart_ipc_status_ping(
    loop_handle: &LoopHandle<'static, WaylandState>,
    wm: &crate::wm::Wm,
) -> Option<crate::ipc::IpcServer> {
    crate::runtime::run_startup_commands(wm);
    crate::wayland::common::spawn_wayland_smoke_window();
    let ipc_server = crate::ipc::IpcServer::bind().ok();
    crate::runtime::register_ipc_source(loop_handle, &ipc_server);
    let (status_ping, status_ping_source) = calloop::ping::make_ping().expect("status ping");
    crate::bar::status::set_internal_status_ping(status_ping);
    loop_handle
        .insert_source(status_ping_source, |_, _, _| {})
        .expect("failed to insert status ping source");
    ipc_server
}

/// Run the shared Wayland per-tick housekeeping and return detailed outcome.
pub fn event_loop_tick(
    wm: &mut Wm,
    state: &WaylandState,
    ipc_server: &mut Option<crate::ipc::IpcServer>,
) -> crate::runtime::TickResult {
    crate::runtime::event_loop_tick_with_options(
        wm,
        ipc_server,
        crate::runtime::TickOptions {
            defer_layout_while_animations_active: true,
            animations_active: state.has_active_window_animations(),
        },
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum AnimationTick {
    Idle,
    SpaceSynced,
    AnimationAdvanced,
    SpaceSyncedAndAnimationAdvanced,
}

impl AnimationTick {
    pub fn needs_redraw(self) -> bool {
        !matches!(self, AnimationTick::Idle)
    }
}

/// Run compositor-space sync and animation progression in one place.
pub fn process_window_animations(state: &mut WaylandState) -> AnimationTick {
    let space_synced = if state.take_space_sync_pending() {
        state.sync_space_from_globals();
        true
    } else {
        false
    };
    let animation_advanced = if state.has_active_window_animations() {
        state.tick_window_animations();
        true
    } else {
        false
    };

    match (space_synced, animation_advanced) {
        (false, false) => AnimationTick::Idle,
        (true, false) => AnimationTick::SpaceSynced,
        (false, true) => AnimationTick::AnimationAdvanced,
        (true, true) => AnimationTick::SpaceSyncedAndAnimationAdvanced,
    }
}
