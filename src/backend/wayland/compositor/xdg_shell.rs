use smithay::{
    desktop::{
        PopupKeyboardGrab, PopupKind, PopupPointerGrab, PopupUngrabStrategy,
        find_popup_root_surface,
    },
    input::{SeatHandler, pointer::Focus},
    reexports::wayland_server::{Resource, protocol::wl_seat},
    wayland::{
        compositor,
        seat::WaylandFocus,
        selection::{
            SelectionHandler,
            data_device::{DataDeviceHandler, DataDeviceState, set_data_device_focus},
            ext_data_control::{
                DataControlHandler as ExtDataControlHandler,
                DataControlState as ExtDataControlState,
            },
            wlr_data_control::{
                DataControlHandler as WlrDataControlHandler,
                DataControlState as WlrDataControlState,
            },
        },
        shell::xdg::{
            PopupSurface, PositionerState, SurfaceCachedState, ToplevelSurface, XdgShellHandler,
            decoration::XdgDecorationHandler,
        },
    },
};

use super::{focus::KeyboardFocusTarget, state::WaylandState};

impl WaylandState {
    fn xdg_toplevel_wants_floating(&self, surface: &ToplevelSurface) -> bool {
        if surface.parent().is_some() {
            return true;
        }

        compositor::with_states(surface.wl_surface(), |states| {
            let mut guard = states.cached_state.get::<SurfaceCachedState>();
            let current = *guard.current();
            let min = current.min_size;
            let max = current.max_size;

            min.w > 0 && min.h > 0 && (min.w == max.w || min.h == max.h)
        })
    }

    pub(crate) fn apply_xdg_toplevel_floating_policy(&mut self, surface: &ToplevelSurface) {
        let wants_floating = self.xdg_toplevel_wants_floating(surface);
        let Some(win) = self.window_id_for_toplevel(surface) else {
            return;
        };
        let Some(g) = self.globals_mut() else {
            return;
        };
        let Some(client) = g.clients.get_mut(&win) else {
            return;
        };

        if wants_floating && !client.is_floating {
            client.float_geo = client.geo;
            client.is_floating = true;
            g.dirty.layout = true;
            g.dirty.space = true;
        }
    }
}

impl SeatHandler for WaylandState {
    type KeyboardFocus = KeyboardFocusTarget;
    type PointerFocus = super::focus::PointerFocusTarget;
    type TouchFocus = super::focus::PointerFocusTarget;

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
        self.request_render();
    }

    fn led_state_changed(
        &mut self,
        _seat: &smithay::input::Seat<Self>,
        led_state: smithay::input::keyboard::LedState,
    ) {
        if let Some(tx) = &self.runtime.led_state_tx {
            let _ = tx.send(led_state);
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

impl ExtDataControlHandler for WaylandState {
    fn data_control_state(&self) -> &ExtDataControlState {
        &self.ext_data_control_state
    }
}

impl WlrDataControlHandler for WaylandState {
    fn data_control_state(&self) -> &WlrDataControlState {
        &self.wlr_data_control_state
    }
}

impl XdgShellHandler for WaylandState {
    fn xdg_shell_state(&mut self) -> &mut smithay::wayland::shell::xdg::XdgShellState {
        &mut self.xdg_shell_state
    }

    fn ack_configure(
        &mut self,
        _surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        _configure: smithay::wayland::shell::xdg::Configure,
    ) {
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Defer window creation until the surface commits its first buffer.
        if !surface.is_initial_configure_sent() {
            let _ = surface.send_configure();
        }
        self.runtime.pending_toplevels.push(surface);
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        let Some(win) = self.window_id_for_toplevel(&surface) else {
            return;
        };
        let props = self.window_properties(win);
        if let Some(g) = self.globals_mut() {
            crate::client::handle_property_change(g, win, &props);
        }
        self.apply_xdg_toplevel_floating_policy(&surface);
        self.update_foreign_toplevel(win);
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        let Some(win) = self.window_id_for_toplevel(&surface) else {
            return;
        };
        let props = self.window_properties(win);
        if let Some(g) = self.globals_mut() {
            crate::client::handle_property_change(g, win, &props);
        }
        self.apply_xdg_toplevel_floating_policy(&surface);
        self.update_foreign_toplevel(win);
    }

    fn parent_changed(&mut self, surface: ToplevelSurface) {
        self.apply_xdg_toplevel_floating_policy(&surface);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        let kind = smithay::desktop::PopupKind::Xdg(surface);
        let _ = self.popups.track_popup(kind);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        // If the surface was still pending (never committed a buffer),
        // just remove it — no window management state was ever created.
        if let Some(pos) = self
            .runtime
            .pending_toplevels
            .iter()
            .position(|t: &ToplevelSurface| t.wl_surface() == surface.wl_surface())
        {
            self.runtime.pending_toplevels.swap_remove(pos);
            return;
        }

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

        // Recover mon.sel if it was cleared by detach_stack, then re-apply seat focus.
        self.restore_focus_after_overlay();
    }

    fn popup_destroyed(&mut self, _surface: PopupSurface) {
        if let Some(old_id) = self.focused_window() {
            if self.window_index.contains_key(&old_id) {
                self.set_focus(old_id);
            } else {
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
            let monitor_id = g.clients.get(&win).map(|client| client.monitor_id);
            if let Some(client) = g.clients.get_mut(&win) {
                client.is_fullscreen = true;
            }
            for (_id, mon) in g.monitors_iter_mut() {
                if mon.fullscreen == Some(win) {
                    mon.fullscreen = None;
                }
            }
            g.dirty.space = true;
            g.dirty.layout = true;
            if let Some(monitor_id) = monitor_id
                && let Some(mon) = g.monitor_mut(monitor_id)
            {
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
            for (_id, mon) in g.monitors_iter_mut() {
                if mon.fullscreen == Some(win) {
                    mon.fullscreen = None;
                }
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

    fn token_created(
        &mut self,
        _token: smithay::wayland::xdg_activation::XdgActivationToken,
        token_data: smithay::wayland::xdg_activation::XdgActivationTokenData,
    ) -> bool {
        if let Some(surface) = token_data.surface.as_ref()
            && let Some(source_win) = self.window_id_for_surface(surface)
            && let Some(g) = self.globals()
            && let Some(client) = g.clients.get(&source_win)
        {
            let _ = token_data.user_data.insert_if_missing_threadsafe(|| {
                crate::client::LaunchContext {
                    monitor_id: client.monitor_id,
                    tags: client.tags,
                }
            });
        }
        true
    }

    fn request_activation(
        &mut self,
        _token: smithay::wayland::xdg_activation::XdgActivationToken,
        token_data: smithay::wayland::xdg_activation::XdgActivationTokenData,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        if let Some(win) = self.window_id_for_surface(&surface) {
            let launch_context = token_data
                .user_data
                .get::<crate::client::LaunchContext>()
                .copied();
            let activated = self.with_wm_mut_unified(|wm, _state| {
                let g = &mut wm.g;
                if let Some(context) = launch_context
                    && let Some(client) = g.clients.get_mut(&win)
                {
                    client.set_tag_mask(context.tags);
                    g.dirty.layout = true;
                    g.dirty.space = true;
                }

                let should_focus = g
                    .clients
                    .get(&win)
                    .and_then(|client| {
                        g.monitor(client.monitor_id)
                            .map(|mon| client.is_visible(mon.selected_tags()))
                    })
                    .unwrap_or(false);
                if !should_focus {
                    return false;
                }

                let mut ctx = wm.ctx();
                crate::focus::activate_client(&mut ctx, win)
            });
            if activated == Some(true) {
                log::debug!(
                    "xdg_activation: activated window {:?} (app_id: {:?})",
                    win,
                    token_data.app_id
                );
            } else {
                log::warn!(
                    "xdg_activation: failed to activate window {:?} (app_id: {:?})",
                    win,
                    token_data.app_id
                );
            }
        } else {
            log::warn!(
                "xdg_activation: could not find window for surface (app_id: {:?})",
                token_data.app_id
            );
        }
    }
}
