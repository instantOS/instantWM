//! Startup shared by the nested and DRM/KMS Wayland runtimes.

use crate::backend::Backend as WmBackend;
use crate::backend::wayland::WaylandBackend;
use crate::backend::wayland::compositor::WaylandState;
use crate::wm::Wm;
use smithay::backend::egl::EGLDisplay;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::reexports::calloop::LoopHandle;

/// D-Bus session, boxed [`Wm`] with Wayland backend, and
/// [`crate::backend::wayland::bootstrap::init_globals`].
pub(crate) fn create_wayland_wm_boxed() -> Box<Wm> {
    crate::backend::wayland::session::ensure_dbus_session();
    let mut wm = Box::new(Wm::new(WmBackend::new_wayland(WaylandBackend::new())));
    if let Some(wayland) = wm.backend.wayland_data_mut() {
        crate::backend::wayland::bootstrap::init_globals(&mut wm.core, wayland);
    }
    wm
}

/// Attach GLES renderer, dmabuf global, and screencopy protocol (winit and DRM).
pub fn attach_gles_renderer_and_protocols(
    state: &mut WaylandState,
    renderer: &mut GlesRenderer,
    egl_display: Option<&EGLDisplay>,
) {
    state.attach_renderer(renderer);
    let egl_for_dmabuf = egl_display.or_else(|| Some(renderer.egl_context().display()));
    state.init_dmabuf_global(
        ImportDma::dmabuf_formats(renderer).into_iter().collect(),
        egl_for_dmabuf,
    );
    state.init_screencopy_manager();
}

/// Wire the Smithay compositor state into [`WaylandBackend`].
pub fn attach_backend_state(wm: &mut Box<Wm>, state: &mut WaylandState) {
    if let WmBackend::Wayland(data) = &mut wm.backend {
        data.backend.attach_state(state);
    }
}

/// Listening socket, XWayland spawn, and StatusNotifier systray thread — shared by both runtimes.
pub fn setup_listen_socket(
    loop_handle: &LoopHandle<'static, WaylandState>,
    state: &WaylandState,
    wm: &mut Box<Wm>,
) {
    let _socket_name = crate::backend::wayland::session::setup_socket(loop_handle, state);
    crate::backend::wayland::session::spawn_xwayland(state, loop_handle);
    // The compositor claims items' native menu toplevels by PID, so it hands
    // the worker its request slot; see `WaylandState::take_expected_systray_menu_toplevel`.
    let wake = crate::runtime::make_wake_ping(loop_handle);
    wm.start_systray(
        Some(std::sync::Arc::clone(&state.runtime.pending_systray_menu)),
        wake,
    );
}

/// Startup commands, smoke window, IPC listener registration, and status-bar ping source.
pub fn autostart_ipc_status_ping(
    loop_handle: &LoopHandle<'static, WaylandState>,
    wm: &crate::wm::Wm,
) -> Option<crate::ipc::IpcServer> {
    crate::runtime::run_startup_commands(wm);
    crate::backend::wayland::session::spawn_smoke_window();
    let ipc_server = crate::ipc::IpcServer::bind().ok();
    crate::runtime::register_ipc_source(loop_handle, &ipc_server);
    let (status_ping, status_ping_source) = calloop::ping::make_ping().expect("status ping");
    crate::bar::status::set_internal_status_ping(status_ping);
    loop_handle
        .insert_source(status_ping_source, |_, _, _| {})
        .expect("failed to insert status ping source");
    ipc_server
}
