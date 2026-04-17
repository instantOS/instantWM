//! Shared Wayland runtime setup and per-tick logic for all backends.
//!
//! **Startup** — Both the winit (nested) and DRM (standalone) runtimes share
//! the same initial steps: session D-Bus, [`crate::wm::Wm`] construction with
//! [`crate::wayland::common::init_wayland_globals`], the Smithay
//! [`smithay::reexports::calloop::EventLoop`], and [`WaylandState`]. Helpers
//! here bundle the identical tail: listening socket + XWayland + systray
//! runtime, then autostart / smoke window / IPC / status-bar ping registration.
//!
//! **Per tick** — Both runtimes perform the same housekeeping: layout, IPC,
//! monitor configuration, and compositor space synchronisation. Helpers
//! delegate to [`crate::runtime`] with Wayland-specific animation options.

use smithay::reexports::calloop::{EventLoop, LoopHandle};
use smithay::reexports::wayland_server::Display;

use crate::backend::Backend as WmBackend;
use crate::backend::wayland::WaylandBackend;
use crate::backend::wayland::compositor::WaylandState;
use crate::wm::Wm;

/// D-Bus session, boxed [`Wm`] with a Wayland backend, and `init_wayland_globals`.
pub fn create_wayland_wm_boxed() -> Box<Wm> {
    crate::wayland::common::ensure_dbus_session();
    let mut wm = Box::new(Wm::new(WmBackend::new_wayland(WaylandBackend::new())));
    if let Some(wayland) = wm.backend.wayland_data_mut() {
        crate::wayland::common::init_wayland_globals(&mut wm.g, wayland);
    }
    wm
}

/// Create the calloop [`EventLoop`], Wayland [`Display`], and [`WaylandState`].
pub fn new_wayland_event_loop_and_state() -> (EventLoop<'static, WaylandState>, WaylandState) {
    let event_loop = EventLoop::try_new().expect("wayland event loop");
    let loop_handle = event_loop.handle();
    let display = Display::new().expect("wayland display");
    let state = WaylandState::new(display, &loop_handle);
    (event_loop, state)
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

/// Autostart, optional smoke window, IPC listener registration, and status-bar ping source.
pub fn wayland_autostart_ipc_status_ping(
    loop_handle: &LoopHandle<'static, WaylandState>,
) -> Option<crate::ipc::IpcServer> {
    crate::startup::autostart::run_autostart();
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

/// Run compositor-space sync and animation progression in one place.
///
/// Returns `true` when either the space was synchronized or at least one
/// animation tick was processed.
pub fn process_window_animations(state: &mut WaylandState) -> bool {
    let mut changed = false;
    if state.take_space_sync_pending() {
        state.sync_space_from_globals();
        changed = true;
    }
    if state.has_active_window_animations() {
        state.tick_window_animations();
        changed = true;
    }
    changed
}
