use smithay::{
    backend::renderer::ImportDma,
    backend::renderer::utils::on_commit_buffer_handler,
    desktop::{
        LayerSurface as DesktopLayerSurface, PopupKeyboardGrab, PopupKind, PopupPointerGrab,
        PopupUngrabStrategy, WindowSurfaceType, find_popup_root_surface, layer_map_for_output,
    },
    input::{SeatHandler, pointer::Focus},
    output::Output,
    reexports::wayland_server::{Client, Resource, protocol::wl_seat},
    utils::SERIAL_COUNTER,
    wayland::{
        buffer::BufferHandler,
        compositor::CompositorHandler,
        compositor::with_states,
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        output::OutputHandler,
        seat::WaylandFocus,
        selection::{
            SelectionHandler,
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
                set_data_device_focus,
            },
        },
        shell::{
            wlr_layer::{
                Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler,
                WlrLayerShellState,
            },
            xdg::{
                PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler,
                decoration::XdgDecorationHandler,
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
    window::{WindowType, is_unmanaged_x11_overlay},
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
        self.popups.commit(surface);

        if let Some(popup) = self.popups.find_popup(surface)
            && let PopupKind::Xdg(ref popup_surface) = popup
            && !popup_surface.is_initial_configure_sent()
        {
            let _ = popup_surface.send_configure();
        }

        if let Some(window) = self
            .space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(surface))
            .cloned()
        {
            window.on_commit();
        }

        let mut layer_surface = None;
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
                layer_surface = Some(surface.clone());
                break;
            }
        }
        if let Some(surface) = layer_surface {
            focus_layer_if_requested(self, &surface);
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

impl ClientDndGrabHandler for WaylandState {
    fn started(
        &mut self,
        _source: Option<smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource>,
        icon: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
        _seat: smithay::input::Seat<Self>,
    ) {
        self.dnd_icon = icon;
    }

    fn dropped(
        &mut self,
        _icon: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
        _accepted: bool,
        _seat: smithay::input::Seat<Self>,
    ) {
        self.dnd_icon = None;
    }
}
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
        // Tag the dmabuf with the render node so clients know which device to use.
        if let Some(node) = self.render_node {
            dmabuf.set_node(node);
        }

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

/// Focus an overlay window if it's a launcher (dmenu, instantmenu).
///
/// This gives launchers immediate keyboard focus so they can receive
/// input right away.
fn focus_overlay_if_launcher(state: &mut WaylandState, element: &smithay::desktop::Window) {
    // Use the unified window classifier
    let typ = state.classify_window(element);
    if typ != WindowType::Launcher && typ != WindowType::Unmanaged {
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

/// Focus a layer surface if it requests keyboard focus.
fn focus_layer_if_requested(
    state: &mut WaylandState,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
) {
    use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, LayerSurfaceCachedState};
    let interactivity = with_states(surface, |states| {
        states
            .cached_state
            .get::<LayerSurfaceCachedState>()
            .current()
            .keyboard_interactivity
    });

    if interactivity == KeyboardInteractivity::None {
        return;
    }

    let serial = SERIAL_COUNTER.next_serial();
    if let Some(keyboard) = state.seat.get_keyboard() {
        keyboard.set_focus(
            state,
            Some(KeyboardFocusTarget::WlSurface(surface.clone())),
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
                self.space.map_element(existing.clone(), geo.loc, false);
                self.space.raise_element(&existing, false);
                focus_overlay_if_launcher(self, &existing);
                trigger_pointer_focus_update(self);
            } else {
                let element = smithay::desktop::Window::new_x11_window(window);
                self.space.map_element(element.clone(), geo.loc, false);
                self.space.raise_element(&element, false);
                focus_overlay_if_launcher(self, &element);
                trigger_pointer_focus_update(self);
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
        self.space.map_element(element.clone(), geo.loc, false);
        self.window_index.insert(win, element);
        self.ensure_client_for_window(win);
        if let Some(g) = self.globals_mut()
            && let Some(c) = g.clients.get_mut(&win)
        {
            c.geo.x = geo.loc.x;
            c.geo.y = geo.loc.y;
            c.geo.w = geo.size.w.max(1);
            c.geo.h = geo.size.h.max(1);
            c.float_geo = c.geo;
            c.name = window.title();
        }
        let _ = window.configure(Some(geo));
        if let Some(g) = self.globals_mut() {
            g.dirty.layout = true;
            g.dirty.space = true;
        }
        self.create_foreign_toplevel(win);
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
        self.space.map_element(element.clone(), geo.loc, false);
        self.space.raise_element(&element, false);
        focus_overlay_if_launcher(self, &element);
        trigger_pointer_focus_update(self);
    }

    fn unmapped_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
    ) {
        let window_id = window.window_id();
        let was_focused = self.is_x11_surface_focused(window_id);

        // Clear seat focus from the dying surface *before* unmapping so
        // the Smithay seat never holds a dead target.
        if was_focused {
            self.clear_seat_focus();
        }

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
        trigger_pointer_focus_update(self);
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
        let was_focused = self.is_x11_surface_focused(window_id);

        // Clear seat focus from the dying surface *before* cleanup.
        if was_focused {
            self.clear_seat_focus();
        }

        if let Some(win) = self.window_id_for_x11_surface(&window) {
            self.remove_window_tracking(win);
            let Some(g) = self.globals_mut() else {
                return;
            };
            g.detach(win);
            g.detach_stack(win);
            g.clients.remove(&win);
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
        trigger_pointer_focus_update(self);

        // Recover mon.sel if it was cleared by detach_stack, then
        // re-apply seat focus.
        self.restore_focus_after_overlay();
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
                self.space.map_element(element.clone(), geometry.loc, false);
                self.space.raise_element(&element, false);
            }
            return;
        };
        if let Some(g) = self.globals_mut()
            && let Some(c) = g.clients.get_mut(&win)
        {
            c.geo.x = geometry.loc.x;
            c.geo.y = geometry.loc.y;
            c.geo.w = geometry.size.w.max(1);
            c.geo.h = geometry.size.h.max(1);
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
        let props = self.window_properties(win);
        if let Some(g) = self.globals_mut() {
            if let Some(client) = g.clients.get_mut(&win) {
                client.name = props.title.clone();
            }
            crate::client::apply_rules(g, win, &props);
        }
        self.update_foreign_toplevel(win);
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        let Some(win) = self.window_id_for_toplevel(&surface) else {
            return;
        };
        let props = self.window_properties(win);
        if let Some(g) = self.globals_mut() {
            crate::client::apply_rules(g, win, &props);
        }
        self.update_foreign_toplevel(win);
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
        {
            let Some(g) = self.globals_mut() else {
                return;
            };
            g.detach(win);
            g.detach_stack(win);
            g.clients.remove(&win);
            g.dirty.layout = true;
            g.dirty.space = true;
        }

        // Recover mon.sel if it was cleared by detach_stack (walks the
        // stack for the next visible window), then re-apply seat focus.
        self.restore_focus_after_overlay();
    }

    fn popup_destroyed(&mut self, _surface: PopupSurface) {
        // When a popup is destroyed, restore focus to the previously focused window.
        // This handles rofi, dmenu, and other XDG popups.
        if let Some(old_id) = self.focused_window() {
            if self.window_index.contains_key(&old_id) {
                self.set_focus(old_id);
            } else {
                // The previously focused window is gone, try to find a valid target
                self.restore_focus_after_overlay();
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
                if g.drag.interactive.active {
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
                g.drag.interactive = crate::globals::DragInteraction {
                    active: true,
                    win,
                    button: crate::types::MouseButton::Left,
                    dragging: false,
                    drag_type: crate::globals::DragType::Move,
                    was_focused: sel == Some(win),
                    was_hidden,
                    start_x: root_x,
                    start_y: root_y,
                    win_start_geo: geo,
                    drop_restore_geo: geo,
                    last_root_x: root_x,
                    last_root_y: root_y,
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
        if let Some(win) = self.window_id_for_toplevel(&surface)
            && let Some(g) = self.globals_mut()
        {
            if let Some(client) = g.clients.get_mut(&win) {
                client.is_fullscreen = true;
            }
            g.dirty.space = true;
            g.dirty.layout = true;
            if let Some(mon) = g.selected_monitor_mut_opt() {
                mon.fullscreen = Some(win);
            }
        }
        surface.with_pending_state(|state| {
            state.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen);
        });
        surface.send_configure();
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(win) = self.window_id_for_toplevel(&surface)
            && let Some(g) = self.globals_mut()
        {
            if let Some(client) = g.clients.get_mut(&win) {
                client.is_fullscreen = false;
            }
            g.dirty.space = true;
            g.dirty.layout = true;
            if let Some(mon) = g.selected_monitor_mut_opt()
                && mon.fullscreen == Some(win)
            {
                mon.fullscreen = None;
            }
        }
        surface.with_pending_state(|state| {
            state.states.unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen);
        });
        surface.send_configure();
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(win) = self.window_id_for_toplevel(&surface)
            && let Some(g) = self.globals_mut()
        {
            let is_currently_floating = g.clients.get(&win).map(|c| c.is_floating).unwrap_or(false);

            if let Some(client) = g.clients.get_mut(&win) {
                if !is_currently_floating {
                    client.float_geo = client.geo;
                }
                client.is_floating = true;
            }
            g.dirty.space = true;
            g.dirty.layout = true;
            if let Some(mon) = g.selected_monitor_mut_opt() {
                mon.fullscreen = Some(win);
            }
        }
        surface.with_pending_state(|state| {
            state.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
        });
        surface.send_configure();
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(win) = self.window_id_for_toplevel(&surface)
            && let Some(g) = self.globals_mut()
        {
            if let Some(client) = g.clients.get_mut(&win) {
                client.is_floating = false;
            }
            g.dirty.space = true;
            g.dirty.layout = true;
            if let Some(mon) = g.selected_monitor_mut_opt()
                && mon.fullscreen == Some(win)
            {
                mon.fullscreen = None;
            }
        }
        surface.with_pending_state(|state| {
            state.states.unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
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

impl smithay::wayland::xdg_activation::XdgActivationHandler for WaylandState {
    fn activation_state(&mut self) -> &mut smithay::wayland::xdg_activation::XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn request_activation(
        &mut self,
        _token: smithay::wayland::xdg_activation::XdgActivationToken,
        token_data: smithay::wayland::xdg_activation::XdgActivationTokenData,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        // Find the window associated with this surface and focus it.
        // We update mon.sel here because activation is an explicit user
        // intent (e.g. from another app) and the WM should reflect it.
        if let Some(win) = self.window_id_for_surface(&surface) {
            let monitor_id = self.globals().and_then(|g| g.clients.monitor_id(win));
            if let Some(g) = self.globals_mut()
                && let Some(mon_id) = monitor_id
                && let Some(mon) = g.monitor_mut(mon_id)
            {
                mon.sel = Some(win);
            }
            self.set_focus(win);
            log::debug!(
                "xdg_activation: activated window (app_id: {:?})",
                token_data.app_id
            );
        } else {
            log::warn!(
                "xdg_activation: could not find window for surface (app_id: {:?})",
                token_data.app_id
            );
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

impl XWaylandKeyboardGrabHandler for WaylandState {
    fn keyboard_focus_for_xsurface(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<Self::KeyboardFocus> {
        let win = self.window_id_for_surface(surface)?;
        let window = self.window_index.get(&win)?;
        Some(KeyboardFocusTarget::Window(window.clone()))
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
        let wl_surface = surface.wl_surface();

        // Check if the keyboard is focused on this layer surface before we destroy it
        let keyboard_focused_on_layer = self
            .seat
            .get_keyboard()
            .and_then(|k| k.current_focus())
            .is_some_and(|focus| {
                if let KeyboardFocusTarget::WlSurface(s) = focus {
                    s == *wl_surface
                } else {
                    false
                }
            });

        for output in self.space.outputs().cloned().collect::<Vec<_>>() {
            let mut map = layer_map_for_output(&output);
            let layers: Vec<_> = map
                .layers()
                .filter(|l| l.wl_surface() == wl_surface)
                .cloned()
                .collect();
            for layer in layers {
                map.unmap_layer(&layer);
            }
        }

        // If the keyboard was focused on this layer surface, clear seat focus
        // and restore it to the WM's selected window.
        if keyboard_focused_on_layer {
            self.clear_seat_focus();
        }

        // Restore seat focus to mon.sel (the WM's selected window).
        // Layer surfaces are not managed windows, so mon.sel should still be
        // valid. We just need to re-apply seat focus.
        self.restore_focus_after_overlay();
    }
}

impl OutputHandler for WaylandState {}

impl smithay::wayland::foreign_toplevel_list::ForeignToplevelListHandler for WaylandState {
    fn foreign_toplevel_list_state(
        &mut self,
    ) -> &mut smithay::wayland::foreign_toplevel_list::ForeignToplevelListState {
        &mut self.foreign_toplevel_list_state
    }
}

smithay::delegate_foreign_toplevel_list!(WaylandState);

/// Trigger a pointer focus update to ensure hover state is correct.
fn trigger_pointer_focus_update(state: &mut WaylandState) {
    let pointer_handle = state.seat.get_pointer();
    let keyboard_handle = state.seat.get_keyboard();
    if let (Some(pointer), Some(keyboard)) = (pointer_handle, keyboard_handle) {
        state.with_wm_mut_unified(|wm, state| {
            crate::wayland::input::pointer::motion::dispatch_pointer_motion(
                wm, state, &pointer, &keyboard,
                0, // time doesn't strictly matter for forced update
            );
        });
    }
}
