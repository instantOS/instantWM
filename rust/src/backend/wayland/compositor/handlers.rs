use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    input::SeatHandler,
    reexports::wayland_server::{protocol::wl_seat, Client},
    utils::SERIAL_COUNTER,
    wayland::{
        buffer::BufferHandler,
        compositor::CompositorHandler,
        output::OutputHandler,
        seat::WaylandFocus,
        selection::{
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
            },
            SelectionHandler,
        },
        shell::xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler},
        shm::ShmHandler,
        xwayland_shell::XWaylandShellHandler,
    },
    xwayland::XwmHandler,
};

use super::{
    focus::{KeyboardFocusTarget, PointerFocusTarget},
    state::{detach_client_from_monitor, WaylandClientState, WaylandState, WindowIdMarker},
};

impl CompositorHandler for WaylandState {
    fn compositor_state(&mut self) -> &mut smithay::wayland::compositor::CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(
        &self,
        client: &'a Client,
    ) -> &'a smithay::wayland::compositor::CompositorClientState {
        &client
            .get_data::<WaylandClientState>()
            .expect("client missing WaylandClientState")
            .compositor_state
    }

    fn commit(
        &mut self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        on_commit_buffer_handler::<Self>(surface);
        let _ = self.popups.commit(surface);
        if let Some(window) = self
            .space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(surface))
            .cloned()
        {
            window.on_commit();
        }
    }
}

impl SelectionHandler for WaylandState {
    type SelectionUserData = ();
}

impl DataDeviceHandler for WaylandState {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for WaylandState {}
impl ServerDndGrabHandler for WaylandState {
    fn send(
        &mut self,
        _mime_type: String,
        _fd: std::os::unix::io::OwnedFd,
        _seat: smithay::input::Seat<Self>,
    ) {
    }
}

impl ShmHandler for WaylandState {
    fn shm_state(&self) -> &smithay::wayland::shm::ShmState {
        &self.shm_state
    }
}

impl BufferHandler for WaylandState {
    fn buffer_destroyed(
        &mut self,
        _buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    ) {
    }
}

impl XwmHandler for WaylandState {
    fn xwm_state(&mut self, _xwm: smithay::xwayland::xwm::XwmId) -> &mut smithay::xwayland::X11Wm {
        self.xwm.as_mut().expect("XWayland is not initialized")
    }

    fn new_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
    ) {
    }

    fn new_override_redirect_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
    ) {
    }

    fn map_window_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
    ) {
    }

    fn mapped_override_redirect_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
    ) {
    }

    fn unmapped_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
    ) {
    }

    fn destroyed_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
    ) {
    }

    fn configure_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
        _x: Option<i32>,
        _y: Option<i32>,
        _w: Option<u32>,
        _h: Option<u32>,
        _reorder: Option<smithay::xwayland::xwm::Reorder>,
    ) {
    }

    fn configure_notify(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
        _geometry: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
        _above: Option<smithay::xwayland::xwm::X11Window>,
    ) {
    }

    fn resize_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
        _button: u32,
        _resize_edge: smithay::xwayland::xwm::ResizeEdge,
    ) {
    }

    fn move_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
        _button: u32,
    ) {
    }
}

impl SeatHandler for WaylandState {
    type KeyboardFocus = KeyboardFocusTarget;
    type PointerFocus = PointerFocusTarget;
    type TouchFocus = PointerFocusTarget;

    fn seat_state(&mut self) -> &mut smithay::input::SeatState<WaylandState> {
        &mut self.seat_state
    }

    fn focus_changed(
        &mut self,
        _seat: &smithay::input::Seat<Self>,
        _target: Option<&KeyboardFocusTarget>,
    ) {
        // TODO: update data device focus for clipboard bridging.
    }

    fn cursor_image(
        &mut self,
        _seat: &smithay::input::Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
        // TODO: store cursor image for rendering.
    }
}

impl XdgShellHandler for WaylandState {
    fn xdg_shell_state(&mut self) -> &mut smithay::wayland::shell::xdg::XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let _ = self.map_new_toplevel(surface);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        let kind = smithay::desktop::PopupKind::Xdg(surface);
        let _ = self.popups.track_popup(kind.clone());
        let _ = self.popups.commit(kind.wl_surface());
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface();
        let windows = self.space.elements().cloned().collect::<Vec<_>>();
        let mut destroyed_win: Option<crate::types::WindowId> = None;
        if let Some(window) = windows
            .into_iter()
            .find(|w| w.wl_surface().as_deref() == Some(wl_surface))
        {
            self.space.unmap_elem(&window);
            destroyed_win = window.user_data().get::<WindowIdMarker>().map(|m| m.0);
        }
        let Some(win) = destroyed_win else { return };
        self.last_configured_size.remove(&win);
        let new_sel = {
            let Some(g) = self.globals_mut() else {
                return;
            };
            if g.clients.contains_key(&win) {
                detach_client_from_monitor(g, win);
                g.clients.remove(&win);
                g.client_list.retain(|id| *id != win.0 as usize);
            }
            g.selected_win()
        };
        // Update Smithay keyboard focus to match mon.sel.
        if let Some(new_win) = new_sel {
            self.set_focus(new_win);
        } else {
            let serial = SERIAL_COUNTER.next_serial();
            if let Some(keyboard) = self.seat.get_keyboard() {
                keyboard.set_focus(self, None::<KeyboardFocusTarget>, serial);
            }
        }
    }

    fn grab(
        &mut self,
        _surface: PopupSurface,
        _seat: wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
    ) {
        // let kind = PopupKind::Xdg(surface.clone());
        // if let Some(parent) = surface.get_parent_surface() {
        //     if let Some(parent_kind) = self.popups.find_popup(&parent) {
        //         let _ = self.popups.grab_popup(kind, parent_kind, &self.seat, _serial);
        //     }
        // }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        _positioner: PositionerState,
        token: u32,
    ) {
        surface.send_repositioned(token);
    }

    fn move_request(
        &mut self,
        surface: ToplevelSurface,
        _seat: wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
    ) {
        if let Some(win) = self.window_id_for_toplevel(&surface) {
            self.set_focus(win);
            self.raise_window(win);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        _seat: wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
        _edges: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    ) {
        if let Some(win) = self.window_id_for_toplevel(&surface) {
            self.set_focus(win);
            self.raise_window(win);
        }
    }
}

impl XWaylandShellHandler for WaylandState {
    fn xwayland_shell_state(
        &mut self,
    ) -> &mut smithay::wayland::xwayland_shell::XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl OutputHandler for WaylandState {}
