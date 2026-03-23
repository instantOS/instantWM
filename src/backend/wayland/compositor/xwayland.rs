use smithay::{utils::SERIAL_COUNTER, xwayland::XwmHandler};

use super::{
    focus::KeyboardFocusTarget,
    state::{WaylandState, WindowIdMarker},
    window::{WindowType, is_unmanaged_x11_overlay},
};

/// Focus an overlay window if it's a launcher (dmenu, instantmenu).
///
/// This gives launchers immediate keyboard focus so they can receive
/// input right away.
pub(super) fn focus_overlay_if_launcher(
    state: &mut WaylandState,
    element: &smithay::desktop::Window,
) {
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

/// Trigger a pointer focus update to ensure hover state is correct.
pub(super) fn trigger_pointer_focus_update(state: &mut WaylandState) {
    let pointer_handle = state.seat.get_pointer();
    let keyboard_handle = state.seat.get_keyboard();
    if let (Some(pointer), Some(keyboard)) = (pointer_handle, keyboard_handle) {
        crate::wayland::input::pointer::motion::dispatch_pointer_motion(
            state, &pointer, &keyboard, 0,
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
                focus_overlay_if_launcher(&mut *self, &existing);
                trigger_pointer_focus_update(&mut *self);
            } else {
                let element = smithay::desktop::Window::new_x11_window(window);
                self.space.map_element(element.clone(), geo.loc, false);
                self.space.raise_element(&element, false);
                focus_overlay_if_launcher(&mut *self, &element);
                trigger_pointer_focus_update(&mut *self);
            }
            return;
        }
        if let Some(win) = self.window_id_for_x11_surface(&window) {
            self.map_window(win);
            if !self.has_layer_keyboard_focus() {
                self.set_focus(win);
            }
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
        {
            let g = &mut self.wm.g;
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
        {
            let g = &mut self.wm.g;
            g.dirty.layout = true;
            g.dirty.space = true;
        }
        self.create_foreign_toplevel(win);
        if !self.has_layer_keyboard_focus() {
            self.set_focus(win);
        }
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
        focus_overlay_if_launcher(&mut *self, &element);
        trigger_pointer_focus_update(&mut *self);
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
        trigger_pointer_focus_update(&mut *self);
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
            let g = &mut self.wm.g;
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
        trigger_pointer_focus_update(&mut *self);

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
        {
            let g = &mut self.wm.g;
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
