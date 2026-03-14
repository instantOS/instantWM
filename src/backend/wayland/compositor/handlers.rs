use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    backend::renderer::ImportDma,
    desktop::{
        find_popup_root_surface, layer_map_for_output, LayerSurface as DesktopLayerSurface,
        PopupKeyboardGrab, PopupKind, PopupPointerGrab, PopupUngrabStrategy, WindowSurfaceType,
    },
    input::{pointer::Focus, SeatHandler},
    output::Output,
    reexports::wayland_server::{protocol::wl_seat, Client, Resource},
    utils::SERIAL_COUNTER,
    wayland::{
        buffer::BufferHandler,
        compositor::with_states,
        compositor::CompositorHandler,
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        output::OutputHandler,
        seat::WaylandFocus,
        selection::{
            data_device::{
                set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState,
                ServerDndGrabHandler,
            },
            SelectionHandler,
        },
        shell::{
            wlr_layer::{
                Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler,
                WlrLayerShellState,
            },
            xdg::{
                decoration::XdgDecorationHandler, PopupSurface, PositionerState, ToplevelSurface,
                XdgShellHandler,
            },
        },
        shm::ShmHandler,
        xwayland_keyboard_grab::XWaylandKeyboardGrabHandler,
        xwayland_shell::XWaylandShellHandler,
    },
    xwayland::{XWaylandClientData, XwmHandler},
};

use super::{
    focus::{KeyboardFocusTarget, PointerFocusTarget},
    state::{WaylandClientState, WaylandState, WindowIdMarker},
};

impl CompositorHandler for WaylandState {
    fn compositor_state(&mut self) -> &mut smithay::wayland::compositor::CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(
        &self,
        client: &'a Client,
    ) -> &'a smithay::wayland::compositor::CompositorClientState {
        if let Some(data) = client.get_data::<WaylandClientState>() {
            &data.compositor_state
        } else if let Some(data) = client.get_data::<XWaylandClientData>() {
            &data.compositor_state
        } else {
            panic!("client missing compositor client state");
        }
    }

    fn commit(
        &mut self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        on_commit_buffer_handler::<Self>(surface);
        let _ = self.popups.commit(surface);

        if let Some(popup) = self.popups.find_popup(surface) {
            if let PopupKind::Xdg(ref popup_surface) = popup {
                if !popup_surface.is_initial_configure_sent() {
                    let _ = popup_surface.send_configure();
                }
            }
        }

        if let Some(window) = self
            .space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(surface))
            .cloned()
        {
            window.on_commit();
        }

        for output in self.space.outputs() {
            let mut map = layer_map_for_output(output);
            if let Some(layer) = map
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .cloned()
            {
                map.arrange();
                let initial_configure_sent = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<LayerSurfaceData>()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .initial_configure_sent
                });
                if !initial_configure_sent {
                    layer.layer_surface().send_configure();
                }
                break;
            }
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

impl DmabufHandler for WaylandState {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
        notifier: ImportNotifier,
    ) {
        let imported = self
            .renderer_mut()
            .and_then(|renderer| renderer.import_dmabuf(&dmabuf, None).ok())
            .is_some();
        if imported {
            let _ = notifier.successful::<Self>();
        } else {
            notifier.failed();
        }
    }
}

/// Classify an X11 surface as an "overlay" (override-redirect, popup, menu,
/// dmenu/instantmenu) at map time so we can cache the result and avoid
/// repeated string scans on every raise.
fn is_unmanaged_x11_overlay(x11: &smithay::xwayland::X11Surface) -> bool {
    if x11.is_override_redirect() || x11.is_popup() || x11.is_transient_for().is_some() {
        return true;
    }
    if matches!(
        x11.window_type(),
        Some(
            smithay::xwayland::xwm::WmWindowType::DropdownMenu
                | smithay::xwayland::xwm::WmWindowType::Menu
                | smithay::xwayland::xwm::WmWindowType::PopupMenu
                | smithay::xwayland::xwm::WmWindowType::Tooltip
                | smithay::xwayland::xwm::WmWindowType::Notification
                | smithay::xwayland::xwm::WmWindowType::Toolbar
                | smithay::xwayland::xwm::WmWindowType::Utility
        )
    ) {
        return true;
    }
    is_launcher_x11_surface(x11)
}

fn is_launcher_x11_surface(x11: &smithay::xwayland::X11Surface) -> bool {
    let class = x11.class().to_ascii_lowercase();
    let instance = x11.instance().to_ascii_lowercase();
    let title = x11.title().to_ascii_lowercase();
    class.contains("dmenu")
        || class.contains("instantmenu")
        || instance.contains("dmenu")
        || instance.contains("instantmenu")
        || title.contains("dmenu")
        || title.contains("instantmenu")
}

fn focus_overlay_if_launcher(state: &mut WaylandState, element: &smithay::desktop::Window) {
    if !element
        .x11_surface()
        .as_ref()
        .is_some_and(|x11| is_launcher_x11_surface(x11))
    {
        return;
    }

    let serial = SERIAL_COUNTER.next_serial();
    if let Some(keyboard) = state.seat.get_keyboard() {
        keyboard.set_focus(
            state,
            Some(KeyboardFocusTarget::Window(element.clone())),
            serial,
        );
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
        window: smithay::xwayland::X11Surface,
    ) {
        let _ = window.set_mapped(true);
        if is_unmanaged_x11_overlay(&window) {
            let window_id = window.window_id();
            let geo = window.geometry();
            let existing = self
                .space
                .elements()
                .find(|w| {
                    w.x11_surface()
                        .is_some_and(|x11| x11.window_id() == window_id)
                })
                .cloned();
            if let Some(existing) = existing {
                self.space.map_element(existing.clone(), geo.loc, true);
                self.space.raise_element(&existing, true);
                focus_overlay_if_launcher(self, &existing);
            } else {
                let element = smithay::desktop::Window::new_x11_window(window);
                self.space.map_element(element.clone(), geo.loc, true);
                self.space.raise_element(&element, true);
                focus_overlay_if_launcher(self, &element);
            }
            return;
        }
        if let Some(win) = self.window_id_for_x11_surface(&window) {
            self.map_window(win);
            self.set_focus(win);
            self.raise_window(win);
            return;
        }

        let element = smithay::desktop::Window::new_x11_window(window.clone());
        let win = self.alloc_window_id();
        let is_overlay = is_unmanaged_x11_overlay(&window);
        let _ = element
            .user_data()
            .get_or_insert_threadsafe(|| WindowIdMarker {
                id: win,
                is_overlay,
            });
        let geo = window.geometry();
        self.space.map_element(element.clone(), geo.loc, true);
        self.window_index.insert(win, element);
        self.ensure_client_for_window(win);
        if let Some(g) = self.globals_mut() {
            if let Some(c) = g.clients.get_mut(&win) {
                c.geo.x = geo.loc.x;
                c.geo.y = geo.loc.y;
                c.geo.w = geo.size.w.max(1);
                c.geo.h = geo.size.h.max(1);
                c.float_geo = c.geo;
                c.name = window.title();
            }
        }
        let _ = window.configure(Some(geo));
        if let Some(g) = self.globals_mut() {
            g.layout_dirty = true;
            g.space_dirty = true;
        }
        self.set_focus(win);
        self.raise_window(win);
    }

    fn mapped_override_redirect_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
    ) {
        let geo = window.geometry();
        let element = smithay::desktop::Window::new_x11_window(window);
        self.space.map_element(element.clone(), geo.loc, true);
        self.space.raise_element(&element, true);
        focus_overlay_if_launcher(self, &element);
    }

    fn unmapped_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
    ) {
        let window_id = window.window_id();
        let was_focused = self
            .seat
            .get_keyboard()
            .and_then(|k| k.current_focus())
            .is_some_and(|focus| {
                if let KeyboardFocusTarget::Window(w) = focus {
                    w.x11_surface()
                        .is_some_and(|x11| x11.window_id() == window_id)
                } else {
                    false
                }
            });

        if let Some(win) = self.window_id_for_x11_surface(&window) {
            self.unmap_window(win);
        } else {
            let element = self
                .space
                .elements()
                .find(|w| {
                    w.x11_surface()
                        .is_some_and(|x11| x11.window_id() == window_id)
                })
                .cloned();
            if let Some(element) = element {
                self.space.unmap_elem(&element);
            }
        }
        if !window.is_override_redirect() {
            let _ = window.set_mapped(false);
        }

        if was_focused {
            self.restore_focus_after_overlay();
        }
    }

    fn destroyed_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
    ) {
        let window_id = window.window_id();
        let is_overlay = self.window_id_for_x11_surface(&window).is_none();
        let was_focused = self
            .seat
            .get_keyboard()
            .and_then(|k| k.current_focus())
            .is_some_and(|focus| {
                if let KeyboardFocusTarget::Window(w) = focus {
                    w.x11_surface()
                        .is_some_and(|x11| x11.window_id() == window_id)
                } else {
                    false
                }
            });

        if let Some(win) = self.window_id_for_x11_surface(&window) {
            self.remove_window_tracking(win);
            let Some(g) = self.globals_mut() else {
                return;
            };
            if g.clients.contains_key(&win) {
                g.detach(win);
                g.detach_stack(win);
                g.clients.remove(&win);
            }
        } else if is_overlay {
            let element = self
                .space
                .elements()
                .find(|w| {
                    w.x11_surface()
                        .is_some_and(|x11| x11.window_id() == window_id)
                })
                .cloned();
            if let Some(element) = element {
                self.space.unmap_elem(&element);
            }
        }

        if was_focused && is_overlay {
            self.restore_focus_after_overlay();
        }
    }

    fn configure_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<smithay::xwayland::xwm::Reorder>,
    ) {
        let mut geo = window.geometry();
        if let Some(x) = x {
            geo.loc.x = x;
        }
        if let Some(y) = y {
            geo.loc.y = y;
        }
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        let _ = window.configure(Some(geo));
    }

    fn configure_notify(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
        geometry: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
        _above: Option<smithay::xwayland::xwm::X11Window>,
    ) {
        let window_id = window.window_id();
        let Some(win) = self.window_id_for_x11_surface(&window) else {
            let element = self
                .space
                .elements()
                .find(|w| {
                    w.x11_surface()
                        .is_some_and(|x11| x11.window_id() == window_id)
                })
                .cloned();
            if let Some(element) = element {
                self.space.map_element(element.clone(), geometry.loc, true);
                self.space.raise_element(&element, true);
            }
            return;
        };
        if let Some(g) = self.globals_mut() {
            if let Some(c) = g.clients.get_mut(&win) {
                c.geo.x = geometry.loc.x;
                c.geo.y = geometry.loc.y;
                c.geo.w = geometry.size.w.max(1);
                c.geo.h = geometry.size.h.max(1);
            }
        }
        self.resize_window(
            win,
            crate::types::Rect {
                x: geometry.loc.x,
                y: geometry.loc.y,
                w: geometry.size.w.max(1),
                h: geometry.size.h.max(1),
            },
        );
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
        window: smithay::xwayland::X11Surface,
        _button: u32,
    ) {
        if let Some(win) = self.window_id_for_x11_surface(&window) {
            self.set_focus(win);
            self.raise_window(win);
        }
    }

    fn disconnected(&mut self, _xwm: smithay::xwayland::xwm::XwmId) {
        self.xwm = None;
        self.xdisplay = None;
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
        seat: &smithay::input::Seat<Self>,
        target: Option<&KeyboardFocusTarget>,
    ) {
        let wl_surface = target.and_then(WaylandFocus::wl_surface);
        let client = wl_surface.and_then(|s| self.display_handle.get_client(s.id()).ok());
        set_data_device_focus(&self.display_handle, seat, client);
    }

    fn cursor_image(
        &mut self,
        _seat: &smithay::input::Seat<Self>,
        image: smithay::input::pointer::CursorImageStatus,
    ) {
        self.cursor_image_status = image;
    }

    fn led_state_changed(
        &mut self,
        _seat: &smithay::input::Seat<Self>,
        led_state: smithay::input::keyboard::LedState,
    ) {
        if let Some(tx) = &self.led_state_tx {
            let _ = tx.send(led_state);
        }
    }
}

impl XdgShellHandler for WaylandState {
    fn xdg_shell_state(&mut self) -> &mut smithay::wayland::shell::xdg::XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let _ = self.map_new_toplevel(surface);
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        let Some(win) = self.window_id_for_toplevel(&surface) else {
            return;
        };
        let title = self.window_title(win);
        if let Some(g) = self.globals_mut() {
            if let Some(client) = g.clients.get_mut(&win) {
                client.name = title.unwrap_or_default();
            }
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        let kind = smithay::desktop::PopupKind::Xdg(surface);
        let _ = self.popups.track_popup(kind);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let Some(win) = self.window_id_for_toplevel(&surface) else {
            return;
        };
        self.remove_window_tracking(win);
        let new_sel = {
            let Some(g) = self.globals_mut() else {
                return;
            };
            if g.clients.contains_key(&win) {
                g.detach(win);
                g.detach_stack(win);
                g.clients.remove(&win);
            }
            g.layout_dirty = true;
            g.space_dirty = true;
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
        surface: PopupSurface,
        _seat: wl_seat::WlSeat,
        serial: smithay::utils::Serial,
    ) {
        let kind = PopupKind::Xdg(surface);
        let root_surface = match find_popup_root_surface(&kind) {
            Ok(s) => s,
            Err(_) => return,
        };
        let root = match self
            .space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(&root_surface))
            .cloned()
        {
            Some(w) => KeyboardFocusTarget::Window(w),
            None => return,
        };

        let mut grab = match self.popups.grab_popup(root, kind, &self.seat, serial) {
            Ok(g) => g,
            Err(_) => return,
        };

        if let Some(keyboard) = self.seat.get_keyboard() {
            if keyboard.is_grabbed()
                && !(keyboard.has_grab(serial)
                    || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        if let Some(pointer) = self.seat.get_pointer() {
            if pointer.is_grabbed()
                && !(pointer.has_grab(serial)
                    || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Clear);
        }
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
            let pointer = self.pointer.current_location();
            let root_x = pointer.x.round() as i32;
            let root_y = pointer.y.round() as i32;
            if let Some(g) = self.globals_mut() {
                if g.drag.title.active {
                    return;
                }
                let Some(client) = g.clients.get(&win) else {
                    return;
                };
                if !client.is_floating {
                    return;
                }
                let geo = client.geo;
                let sel = g.selected_win();
                let was_hidden = client.is_hidden;
                g.drag.title = crate::globals::TitleDragState {
                    active: true,
                    win,
                    button: crate::types::MouseButton::Left,
                    was_focused: sel == Some(win),
                    was_hidden,
                    start_x: root_x,
                    start_y: root_y,
                    win_start_geo: geo,
                    drop_restore_geo: geo,
                    last_root_x: root_x,
                    last_root_y: root_y,
                    dragging: false,
                    suppress_click_action: true,
                };
            }
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

    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        mut _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        if let Some(win) = self.window_id_for_toplevel(&surface) {
            if let Some(g) = self.globals_mut() {
                if let Some(client) = g.clients.get_mut(&win) {
                    client.is_fullscreen = true;
                }
                g.space_dirty = true;
                g.layout_dirty = true;
                if let Some(mon) = g.selected_monitor_mut_opt() {
                    mon.fullscreen = Some(win);
                }
            }
        }
        surface.with_pending_state(|state| {
            state.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen);
        });
        surface.send_configure();
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(win) = self.window_id_for_toplevel(&surface) {
            if let Some(g) = self.globals_mut() {
                if let Some(client) = g.clients.get_mut(&win) {
                    client.is_fullscreen = false;
                }
                g.space_dirty = true;
                g.layout_dirty = true;
                if let Some(mon) = g.selected_monitor_mut_opt() {
                    if mon.fullscreen == Some(win) {
                        mon.fullscreen = None;
                    }
                }
            }
        }
        surface.with_pending_state(|state| {
            state.states.unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen);
        });
        surface.send_configure();
    }
}

impl XdgDecorationHandler for WaylandState {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
        let _ = toplevel.send_configure();
    }

    fn request_mode(
        &mut self,
        toplevel: ToplevelSurface,
        mode: smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    ) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(mode);
        });
        let _ = toplevel.send_configure();
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
        let _ = toplevel.send_configure();
    }
}

impl XWaylandShellHandler for WaylandState {
    fn xwayland_shell_state(
        &mut self,
    ) -> &mut smithay::wayland::xwayland_shell::XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl XWaylandKeyboardGrabHandler for WaylandState {
    fn keyboard_focus_for_xsurface(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<Self::KeyboardFocus> {
        if let Some(win) = self.window_id_for_surface(surface) {
            if let Some(window) = self.window_index.get(&win) {
                return Some(KeyboardFocusTarget::Window(window.clone()));
            }
        }
        self.window_for_surface(surface)
            .map(KeyboardFocusTarget::Window)
    }
}

impl WlrLayerShellHandler for WaylandState {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.wlr_layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let layer_surface = DesktopLayerSurface::new(surface, namespace);
        let target_output = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.space.outputs().next().cloned());
        let Some(target_output) = target_output else {
            return;
        };
        let mut map = layer_map_for_output(&target_output);
        let _ = map.map_layer(&layer_surface);
        map.arrange();
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        for output in self.space.outputs().cloned().collect::<Vec<_>>() {
            let mut map = layer_map_for_output(&output);
            let wl_surface = surface.wl_surface();
            let layers: Vec<_> = map
                .layers()
                .filter(|l| l.wl_surface() == wl_surface)
                .cloned()
                .collect();
            for layer in layers {
                map.unmap_layer(&layer);
            }
        }
    }
}

impl OutputHandler for WaylandState {}
