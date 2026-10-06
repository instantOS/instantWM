//! Startup shared by the nested and DRM/KMS Wayland runtimes.

use crate::backend::wayland::compositor::WaylandState;
use crate::wm::WaylandWm as Wm;
use smithay::backend::egl::EGLDisplay;
use smithay::backend::renderer::ImportDma;
use smithay::reexports::calloop::LoopHandle;

/// D-Bus session, owned [`Wm`] with Wayland backend, and
/// [`crate::backend::wayland::bootstrap::init_globals`].
pub(crate) fn create_wayland_wm() -> Wm {
    crate::backend::wayland::session::ensure_dbus_session();
    let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
    crate::backend::wayland::bootstrap::init_globals(&mut wm.core.state);
    wm
}

/// Attach GLES renderer, dmabuf global, and screencopy protocol (winit and DRM).
pub fn attach_gles_renderer_and_protocols(
    state: &mut WaylandState,
    mut graphics: crate::backend::wayland::compositor::graphics::Graphics,
    egl_display: Option<&EGLDisplay>,
) {
    graphics.with_renderer(|renderer| {
        let egl_for_dmabuf = egl_display.or_else(|| Some(renderer.egl_context().display()));
        state.init_dmabuf_global(
            ImportDma::dmabuf_formats(renderer).into_iter().collect(),
            egl_for_dmabuf,
        );
    });
    state.attach_graphics(graphics);
    state.init_screencopy_manager();
}

/// Listening socket, XWayland spawn, and StatusNotifier systray thread — shared by both runtimes.
pub fn setup_listen_socket(
    loop_handle: &LoopHandle<'static, WaylandState>,
    state: &mut WaylandState,
) {
    let _socket_name = crate::backend::wayland::session::setup_socket(loop_handle, state);
    crate::backend::wayland::session::spawn_xwayland(state, loop_handle);
    // The compositor claims items' native menu toplevels by PID, so it hands
    // the worker its request slot; see `WaylandState::take_expected_systray_menu_toplevel`.
    let wake = crate::runtime::make_wake_ping(loop_handle);
    state.wm.start_systray(
        Some(std::sync::Arc::clone(
            &state.native.runtime.pending_systray_menu,
        )),
        wake,
    );
}

/// Startup commands, IPC listener registration, and status-bar ping source.
pub fn autostart_ipc_status_ping(
    loop_handle: &LoopHandle<'static, WaylandState>,
    wm: &mut crate::wm::WaylandWm,
) -> Option<crate::ipc::IpcServer> {
    crate::runtime::run_startup_commands(wm);
    let ipc_server = crate::ipc::IpcServer::bind().ok();
    crate::runtime::register_ipc_source(loop_handle, &ipc_server);
    if let Some(status_wake) = wm.core.bar.status_sources.take_wake_source() {
        loop_handle
            .insert_source(status_wake, |_, _, _| {})
            .expect("failed to insert status ping source");
    }
    let (slop_ping, slop_ping_source) = calloop::ping::make_ping().expect("slop ping");
    crate::mouse::slop::set_region_selection_ping(slop_ping);
    loop_handle
        .insert_source(slop_ping_source, |_, _, _| {})
        .expect("failed to insert region-selection ping source");
    ipc_server
}
