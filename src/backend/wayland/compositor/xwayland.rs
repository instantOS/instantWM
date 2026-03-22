use smithay::{utils::SERIAL_COUNTER, xwayland::XwmHandler};

use super::{
    focus::KeyboardFocusTarget,
    state::{WaylandRuntime, WaylandState, WindowIdMarker},
    window::{is_unmanaged_x11_overlay, WindowType},
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
            WaylandRuntime::from_state_mut(state),
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
        // SAFETY: wm and state_pointee are disjoint borrows into WaylandState.
        // We only read seat/space/pointer_location through state_pointee and
        // read/write globals through wm. No aliasing occurs.
        let wm = unsafe { &mut *(&mut state.wm as *mut crate::wm::Wm) };
        let state_ref = unsafe { &mut *(state as *mut WaylandState) };
        crate::wayland::input::pointer::motion::dispatch_pointer_motion(
            wm, state_ref, &pointer, &keyboard, 0,
        );
    }
}

impl XwmHandler for WaylandRuntime {
    fn xwm_state(&mut self, _xwm: smithay::xwayland::xwm::XwmId) -> &mut smithay::xwayland::X11Wm {
        self.state
            .xwm
            .as_mut()
            .expect("XWayland is not initialized")
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
                .state
                .space
                .elements()
                .find(|w| {
                    w.x11_surface()
                        .is_some_and(|x11| x11.window_id() == window_id)
                })
                .cloned();
            if let Some(existing) = existing {
                self.state
                    .space
                    .map_element(existing.clone(), geo.loc, false);
                self.state.space.raise_element(&existing, false);
                focus_overlay_if_launcher(&mut self.state, &existing);
                trigger_pointer_focus_update(&mut self.state);
            } else {
                let element = smithay::desktop::Window::new_x11_window(window);
                self.state
                    .space
                    .map_element(element.clone(), geo.loc, false);
                self.state.space.raise_element(&element, false);
                focus_overlay_if_launcher(&mut self.state, &element);
                trigger_pointer_focus_update(&mut self.state);
            }
            return;
        }
        if let Some(win) = self.state.window_id_for_x11_surface(&window) {
            self.state.map_window(win);
            self.state.set_focus(win);
            self.state.raise_window(win);
            return;
        }

        let element = smithay::desktop::Window::new_x11_window(window.clone());
        let win = self.state.alloc_window_id();
        let is_overlay = is_unmanaged_x11_overlay(&window);
        let _ = element
            .user_data()
            .get_or_insert_threadsafe(|| WindowIdMarker {
                id: win,
                is_overlay,
            });
        let geo = window.geometry();
        self.state
            .space
            .map_element(element.clone(), geo.loc, false);
        self.state.window_index.insert(win, element);
        self.state.ensure_client_for_window(win);
        {
            let g = &mut self.state.wm.g;
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
            let g = &mut self.state.wm.g;
            g.dirty.layout = true;
            g.dirty.space = true;
        }
        self.state.create_foreign_toplevel(win);
        self.state.set_focus(win);
        self.state.raise_window(win);
    }

    fn mapped_override_redirect_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
    ) {
        let geo = window.geometry();
        let element = smithay::desktop::Window::new_x11_window(window);
        self.state
            .space
            .map_element(element.clone(), geo.loc, false);
        self.state.space.raise_element(&element, false);
        focus_overlay_if_launcher(&mut self.state, &element);
        trigger_pointer_focus_update(&mut self.state);
    }

    fn unmapped_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
    ) {
        let window_id = window.window_id();
        let was_focused = self.state.is_x11_surface_focused(window_id);

        // Clear seat focus from the dying surface *before* unmapping so
        // the Smithay seat never holds a dead target.
        if was_focused {
            self.state.clear_seat_focus();
        }

        if let Some(win) = self.state.window_id_for_x11_surface(&window) {
            self.state.unmap_window(win);
        } else {
            let element = self
                .state
                .space
                .elements()
                .find(|w| {
                    w.x11_surface()
                        .is_some_and(|x11| x11.window_id() == window_id)
                })
                .cloned();
            if let Some(element) = element {
                self.state.space.unmap_elem(&element);
            }
        }
        trigger_pointer_focus_update(&mut self.state);
        if !window.is_override_redirect() {
            let _ = window.set_mapped(false);
        }

        if was_focused {
            self.state.restore_focus_after_overlay();
        }
    }

    fn destroyed_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
    ) {
        let window_id = window.window_id();
        let is_overlay = self.state.window_id_for_x11_surface(&window).is_none();
        let was_focused = self.state.is_x11_surface_focused(window_id);

        // Clear seat focus from the dying surface *before* cleanup.
        if was_focused {
            self.state.clear_seat_focus();
        }

        if let Some(win) = self.state.window_id_for_x11_surface(&window) {
            self.state.remove_window_tracking(win);
            let g = &mut self.state.wm.g;
            g.detach(win);
            g.detach_stack(win);
            g.clients.remove(&win);
        } else if is_overlay {
            let element = self
                .state
                .space
                .elements()
                .find(|w| {
                    w.x11_surface()
                        .is_some_and(|x11| x11.window_id() == window_id)
                })
                .cloned();
            if let Some(element) = element {
                self.state.space.unmap_elem(&element);
            }
        }
        trigger_pointer_focus_update(&mut self.state);

        // Recover mon.sel if it was cleared by detach_stack, then
        // re-apply seat focus.
        self.state.restore_focus_after_overlay();
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
        let Some(win) = self.state.window_id_for_x11_surface(&window) else {
            let element = self
                .state
                .space
                .elements()
                .find(|w| {
                    w.x11_surface()
                        .is_some_and(|x11| x11.window_id() == window_id)
                })
                .cloned();
            if let Some(element) = element {
                self.state
                    .space
                    .map_element(element.clone(), geometry.loc, false);
                self.state.space.raise_element(&element, false);
            }
            return;
        };
        {
            let g = &mut self.state.wm.g;
            if let Some(c) = g.clients.get_mut(&win) {
                c.geo.x = geometry.loc.x;
                c.geo.y = geometry.loc.y;
                c.geo.w = geometry.size.w.max(1);
                c.geo.h = geometry.size.h.max(1);
            }
        }
        self.state.resize_window(
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
        if let Some(win) = self.state.window_id_for_x11_surface(&window) {
            self.state.set_focus(win);
            self.state.raise_window(win);
        }
    }

    fn disconnected(&mut self, _xwm: smithay::xwayland::xwm::XwmId) {
        self.state.xwm = None;
        self.state.xdisplay = None;
    }
}
