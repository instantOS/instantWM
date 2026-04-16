use smithay::{
    utils::SERIAL_COUNTER,
    xwayland::{X11Surface, XwmHandler, xwm::WmWindowProperty},
};

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
        state.with_wm_mut_unified(|wm, state| {
            crate::wayland::input::pointer::motion::dispatch_pointer_motion(
                wm, state, &pointer, &keyboard,
                0, // time doesn't strictly matter for forced update
            );
        });
    }
}

fn sync_xwayland_surface_metadata(
    state: &mut WaylandState,
    win: crate::types::WindowId,
    surface: &X11Surface,
) {
    let props = crate::client::x11_policy::window_properties_from_x11_surface(surface);
    if let Some(g) = state.globals_mut() {
        crate::client::handle_property_change(g, win, &props);
    }
    state.update_foreign_toplevel(win);
    state.request_bar_redraw();
}

fn apply_xwayland_surface_policy(
    state: &mut WaylandState,
    win: crate::types::WindowId,
    surface: &X11Surface,
) {
    let transient_parent = crate::client::x11_policy::transient_for_window_id(surface)
        .and_then(|parent_x11| state.window_id_for_x11_window(parent_x11.into()));
    let should_float_for_type =
        crate::client::x11_policy::should_float_for_x11_type(surface.window_type());
    let preferred_border = state
        .globals()
        .map(|g| {
            crate::client::x11_policy::preferred_border_width(
                g.cfg.border_width_px,
                surface.is_decorated(),
            )
        })
        .unwrap_or(0);
    let transient_parent_state = state.globals().and_then(|g| {
        transient_parent.and_then(|parent| {
            g.clients
                .get(&parent)
                .map(|parent_client| (parent_client.monitor_id, parent_client.tags))
        })
    });

    let mut border_resize = None;
    let mut changed_layout = false;
    if let Some(g) = state.globals_mut()
        && let Some(client) = g.clients.get_mut(&win)
    {
        crate::client::x11_policy::apply_wm_hints_to_client(client, surface.hints());
        crate::client::x11_policy::apply_size_hints_to_client(client, surface.size_hints());

        if let Some((monitor_id, tags)) = transient_parent_state {
            client.monitor_id = monitor_id;
            client.set_tag_mask(tags);
        }

        if (transient_parent.is_some() || client.is_fixed_size || should_float_for_type)
            && !client.is_floating
        {
            client.float_geo = client.geo;
            client.is_floating = true;
            changed_layout = true;
        }

        client.is_fullscreen = surface.is_fullscreen();
        client.is_hidden = surface.is_hidden();

        if preferred_border != client.border_width {
            let total_w = client.total_width();
            let total_h = client.total_height();
            client.border_width = preferred_border;
            client.old_border_width = preferred_border;
            border_resize = Some(crate::types::Rect {
                x: client.geo.x,
                y: client.geo.y,
                w: (total_w - 2 * preferred_border).max(1),
                h: (total_h - 2 * preferred_border).max(1),
            });
        }
    }

    if let Some(rect) = border_resize {
        state.with_wm_mut_unified(|wm, _state| {
            let mut ctx = wm.ctx();
            if let crate::contexts::WmCtx::Wayland(ctx_wayland) = &mut ctx {
                crate::contexts::WmCtx::Wayland(ctx_wayland.reborrow()).move_resize(
                    win,
                    rect,
                    crate::geometry::MoveResizeOptions::hinted_immediate(false),
                );
            }
        });
    }

    if changed_layout {
        if let Some(g) = state.globals_mut() {
            g.queue_layout_for_client(win);
        }
        state.request_space_sync();
    }
    if let Some(g) = state.globals_mut()
        && let Some(mid) = g.clients.monitor_id(win)
        && let Some(mon) = g.monitor_mut(mid)
    {
        if surface.is_fullscreen() {
            mon.fullscreen = Some(win);
        } else if mon.fullscreen == Some(win) {
            mon.fullscreen = None;
        }
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
            sync_xwayland_surface_metadata(self, win, &window);
            apply_xwayland_surface_policy(self, win, &window);
            self.map_window(win);
            self.activate_and_raise_window(win);
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
        sync_xwayland_surface_metadata(self, win, &window);
        apply_xwayland_surface_policy(self, win, &window);
        if let Some(g) = self.globals_mut() {
            crate::client::sync_client_geometry(
                g,
                win,
                crate::types::Rect {
                    x: geo.loc.x,
                    y: geo.loc.y,
                    w: geo.size.w.max(1),
                    h: geo.size.h.max(1),
                },
            );
        }
        let final_rect = if let Some(rect) = self
            .globals()
            .and_then(|g| crate::client::sane_floating_spawn_rect(g, win))
        {
            if let Some(g) = self.globals_mut() {
                crate::client::sync_client_geometry(g, win, rect);
            }
            self.resize_window(win, rect);
            rect
        } else {
            crate::types::Rect {
                x: geo.loc.x,
                y: geo.loc.y,
                w: geo.size.w.max(1),
                h: geo.size.h.max(1),
            }
        };
        let _ = window.configure(Some(smithay::utils::Rectangle::new(
            (final_rect.x, final_rect.y).into(),
            (final_rect.w.max(1), final_rect.h.max(1)).into(),
        )));
        if let Some(g) = self.globals_mut() {
            g.queue_layout_for_client(win);
        }
        self.request_space_sync();
        self.create_foreign_toplevel(win);
        self.activate_and_raise_window(win);
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
            g.detach_z_order(win);
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

        // Recover mon.sel if it was cleared by detach_z_order, then
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
        if let Some(g) = self.globals_mut() {
            crate::client::sync_client_geometry(
                g,
                win,
                crate::types::Rect {
                    x: geometry.loc.x,
                    y: geometry.loc.y,
                    w: geometry.size.w.max(1),
                    h: geometry.size.h.max(1),
                },
            );
        }
        // This is an acknowledgement/notification from XWayland about the
        // geometry it is already using. Feeding it back into `resize_window`
        // would send another X11 configure and create a resize loop.
        let mode = self.default_window_move_mode();
        self.set_window_target_rect(
            win,
            crate::types::Rect {
                x: geometry.loc.x,
                y: geometry.loc.y,
                w: geometry.size.w.max(1),
                h: geometry.size.h.max(1),
            },
            mode,
        );
    }

    fn property_notify(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
        property: WmWindowProperty,
    ) {
        let Some(win) = self.window_id_for_x11_surface(&window) else {
            return;
        };

        match property {
            WmWindowProperty::Title | WmWindowProperty::Class => {
                sync_xwayland_surface_metadata(self, win, &window);
                apply_xwayland_surface_policy(self, win, &window);
            }
            WmWindowProperty::Hints
            | WmWindowProperty::NormalHints
            | WmWindowProperty::TransientFor
            | WmWindowProperty::WindowType
            | WmWindowProperty::MotifHints
            | WmWindowProperty::StartupId
            | WmWindowProperty::Pid
            | WmWindowProperty::Protocols
            | WmWindowProperty::Opacity => {
                apply_xwayland_surface_policy(self, win, &window);
                if matches!(
                    property,
                    WmWindowProperty::TransientFor | WmWindowProperty::WindowType
                ) {
                    let mut request_space_sync = false;
                    if let Some(g) = self.globals_mut() {
                        g.queue_layout_for_client(win);
                        request_space_sync = true;
                    }
                    if request_space_sync {
                        self.request_space_sync();
                    }
                }
                if matches!(property, WmWindowProperty::Hints) {
                    self.request_bar_redraw();
                }
            }
        }
    }

    fn maximize_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
    ) {
        let Some(win) = self.window_id_for_x11_surface(&window) else {
            return;
        };
        let _ = window.set_maximized(true);
        self.with_wm_mut_unified(|wm, _state| {
            let mut ctx = wm.ctx();
            if let crate::contexts::WmCtx::Wayland(ctx_wayland) = &mut ctx {
                let work_rect = ctx_wayland
                    .core
                    .globals()
                    .clients
                    .monitor_id(win)
                    .and_then(|mid| ctx_wayland.core.globals().monitor(mid))
                    .map(|mon| mon.work_rect);
                if let Some(client) = ctx_wayland.core.globals_mut().clients.get_mut(&win) {
                    client.oldstate = client.is_floating as i32;
                    client.float_geo = client.geo;
                    client.is_floating = true;
                }
                if let Some(work_rect) = work_rect {
                    crate::contexts::WmCtx::Wayland(ctx_wayland.reborrow()).move_resize(
                        win,
                        work_rect,
                        crate::geometry::MoveResizeOptions::hinted_immediate(false),
                    );
                }
            }
        });
    }

    fn unmaximize_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
    ) {
        let Some(win) = self.window_id_for_x11_surface(&window) else {
            return;
        };
        let _ = window.set_maximized(false);
        self.with_wm_mut_unified(|wm, _state| {
            let mut ctx = wm.ctx();
            if let crate::contexts::WmCtx::Wayland(ctx_wayland) = &mut ctx {
                let restore_rect = ctx_wayland
                    .core
                    .globals()
                    .clients
                    .get(&win)
                    .map(|client| client.float_geo);
                if let Some(client) = ctx_wayland.core.globals_mut().clients.get_mut(&win) {
                    client.is_floating = client.oldstate != 0;
                }
                if let Some(restore_rect) = restore_rect {
                    crate::contexts::WmCtx::Wayland(ctx_wayland.reborrow()).move_resize(
                        win,
                        restore_rect,
                        crate::geometry::MoveResizeOptions::hinted_immediate(false),
                    );
                }
            }
        });
    }

    fn fullscreen_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
    ) {
        let Some(win) = self.window_id_for_x11_surface(&window) else {
            return;
        };
        let _ = window.set_fullscreen(true);
        if let Some(g) = self.globals_mut() {
            let monitor_id = g.clients.get(&win).map(|client| client.monitor_id);
            if let Some(client) = g.clients.get_mut(&win) {
                client.is_fullscreen = true;
            }
            for (_id, mon) in g.monitors_iter_mut() {
                if mon.fullscreen == Some(win) {
                    mon.fullscreen = None;
                }
            }
            if let Some(monitor_id) = monitor_id
                && let Some(mon) = g.monitor_mut(monitor_id)
            {
                mon.fullscreen = Some(win);
            }
            g.queue_layout_for_client(win);
        }
        self.request_space_sync();
    }

    fn unfullscreen_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
    ) {
        let Some(win) = self.window_id_for_x11_surface(&window) else {
            return;
        };
        let _ = window.set_fullscreen(false);
        if let Some(g) = self.globals_mut() {
            if let Some(client) = g.clients.get_mut(&win) {
                client.is_fullscreen = false;
            }
            for (_id, mon) in g.monitors_iter_mut() {
                if mon.fullscreen == Some(win) {
                    mon.fullscreen = None;
                }
            }
            g.queue_layout_for_client(win);
        }
        self.request_space_sync();
    }

    fn minimize_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
    ) {
        let Some(win) = self.window_id_for_x11_surface(&window) else {
            return;
        };
        let _ = window.set_hidden(true);
        self.with_wm_mut_unified(|wm, _state| {
            let mut ctx = wm.ctx();
            crate::client::hide(&mut ctx, win);
        });
    }

    fn unminimize_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
    ) {
        let Some(win) = self.window_id_for_x11_surface(&window) else {
            return;
        };
        let _ = window.set_hidden(false);
        self.with_wm_mut_unified(|wm, _state| {
            let mut ctx = wm.ctx();
            crate::client::show_window(&mut ctx, win);
        });
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
            self.activate_and_raise_window(win);
        }
    }

    fn disconnected(&mut self, _xwm: smithay::xwayland::xwm::XwmId) {
        self.xwm = None;
        self.xdisplay = None;
    }
}
