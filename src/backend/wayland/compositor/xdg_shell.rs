use smithay::{
    desktop::{
        PopupKeyboardGrab, PopupKind, PopupPointerGrab, PopupUngrabStrategy,
        find_popup_root_surface,
    },
    input::{
        SeatHandler,
        dnd::{DnDGrab, GrabType, Source},
        pointer::Focus,
    },
    reexports::wayland_server::{Resource, protocol::wl_seat},
    wayland::{
        compositor,
        seat::WaylandFocus,
        selection::{
            SelectionHandler,
            data_device::{
                DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, set_data_device_focus,
            },
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
        let mut changed = false;
        {
            let Some(g) = self.globals_mut() else {
                return;
            };
            let Some(client) = g.clients.get_mut(&win) else {
                return;
            };

            if wants_floating && !client.is_floating {
                client.float_geo = client.geo;
                client.is_floating = true;
                g.queue_layout_for_client(win);
                changed = true;
            }
        }
        if changed {
            self.request_space_sync();
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
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl ExtDataControlHandler for WaylandState {
    fn data_control_state(&mut self) -> &mut ExtDataControlState {
        &mut self.ext_data_control_state
    }
}

impl WlrDataControlHandler for WaylandState {
    fn data_control_state(&mut self) -> &mut WlrDataControlState {
        &mut self.wlr_data_control_state
    }
}

impl WaylandDndGrabHandler for WaylandState {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        icon: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
        seat: smithay::input::Seat<Self>,
        serial: smithay::utils::Serial,
        type_: GrabType,
    ) {
        self.runtime.dnd_icon = icon;
        self.request_render();

        match type_ {
            GrabType::Pointer => {
                let Some(pointer) = seat.get_pointer() else {
                    source.cancel();
                    self.runtime.dnd_icon = None;
                    self.request_render();
                    return;
                };
                let Some(start_data) = pointer.grab_start_data() else {
                    source.cancel();
                    self.runtime.dnd_icon = None;
                    self.request_render();
                    return;
                };

                pointer.set_grab(
                    self,
                    DnDGrab::new_pointer(&self.display_handle, start_data, source, seat),
                    serial,
                    Focus::Keep,
                );
            }
            GrabType::Touch => {
                let Some(touch) = seat.get_touch() else {
                    source.cancel();
                    self.runtime.dnd_icon = None;
                    self.request_render();
                    return;
                };
                let Some(start_data) = touch.grab_start_data() else {
                    source.cancel();
                    self.runtime.dnd_icon = None;
                    self.request_render();
                    return;
                };

                touch.set_grab(
                    self,
                    DnDGrab::new_touch(&self.display_handle, start_data, source, seat),
                    serial,
                );
            }
        }
    }
}

impl smithay::input::dnd::DndGrabHandler for WaylandState {
    fn dropped(
        &mut self,
        _target: Option<smithay::input::dnd::DndTarget<'_, Self>>,
        _validated: bool,
        _seat: smithay::input::Seat<Self>,
        _location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {
        self.runtime.dnd_icon = None;
        self.request_render();
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
        self.request_bar_redraw();
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
        self.request_bar_redraw();
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
            g.queue_layout_for_all_monitors();
        }
        self.request_space_sync();

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
        let mut request_space_sync = false;
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
            g.queue_layout_for_client(win);
            request_space_sync = true;
            if let Some(monitor_id) = monitor_id
                && let Some(mon) = g.monitor_mut(monitor_id)
            {
                mon.fullscreen = Some(win);
            }
        }
        if request_space_sync {
            self.request_space_sync();
        }
        surface.with_pending_state(|state| {
            state.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen);
        });
        surface.send_configure();
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        let mut request_space_sync = false;
        if let Some(win) = self.window_id_for_toplevel(&surface)
            && let Some(g) = self.globals_mut()
        {
            if let Some(client) = g.clients.get_mut(&win) {
                client.is_fullscreen = false;
            }
            g.queue_layout_for_client(win);
            request_space_sync = true;
            for (_id, mon) in g.monitors_iter_mut() {
                if mon.fullscreen == Some(win) {
                    mon.fullscreen = None;
                }
            }
        }
        if request_space_sync {
            self.request_space_sync();
        }
        surface.with_pending_state(|state| {
            state.states.unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen);
        });
        surface.send_configure();
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        let mut request_space_sync = false;
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
            g.queue_layout_for_client(win);
            request_space_sync = true;
            if let Some(mon) = g.selected_monitor_mut_opt() {
                mon.fullscreen = Some(win);
            }
        }
        if request_space_sync {
            self.request_space_sync();
        }
        surface.with_pending_state(|state| {
            state.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
        });
        surface.send_configure();
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        let mut request_space_sync = false;
        if let Some(win) = self.window_id_for_toplevel(&surface)
            && let Some(g) = self.globals_mut()
        {
            if let Some(client) = g.clients.get_mut(&win) {
                client.is_floating = false;
            }
            g.queue_layout_for_client(win);
            request_space_sync = true;
            if let Some(mon) = g.selected_monitor_mut_opt()
                && mon.fullscreen == Some(win)
            {
                mon.fullscreen = None;
            }
        }
        if request_space_sync {
            self.request_space_sync();
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
        if let Some(g) = self.globals() {
            let context = token_data
                .surface
                .as_ref()
                .and_then(|surface| self.window_id_for_surface(surface))
                .and_then(|source_win| g.clients.get(&source_win))
                .map(|client| crate::client::LaunchContext {
                    monitor_id: client.monitor_id,
                    tags: client.tags,
                })
                .unwrap_or_else(|| crate::client::current_launch_context(g));
            let _ = token_data
                .user_data
                .insert_if_missing_threadsafe(|| context);
        }
        true
    }

    fn request_activation(
        &mut self,
        _token: smithay::wayland::xdg_activation::XdgActivationToken,
        token_data: smithay::wayland::xdg_activation::XdgActivationTokenData,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        let launch_context = token_data
            .user_data
            .get::<crate::client::LaunchContext>()
            .copied();
        if let Some(win) = self.window_id_for_surface(&surface) {
            // Check whether the window is already visible on its monitor's
            // currently selected tags.  When it is, we focus it immediately.
            // When it is not (i.e. it lives on a different tag), we mark it
            // as urgent so the bar highlights the tag without stealing focus.
            let is_currently_visible = self
                .globals()
                .and_then(|g| {
                    g.clients
                        .get(&win)
                        .and_then(|c| c.monitor(g).map(|m| c.is_visible(m.selected_tags())))
                })
                .unwrap_or(false);

            let activated = self.with_wm_mut_unified(|wm, _state| {
                let mut ctx = wm.ctx();
                if is_currently_visible {
                    crate::focus::activate_client(&mut ctx, win)
                } else {
                    // Mark as urgent so the bar shows the indicator.
                    if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
                        client.is_urgent = true;
                    }
                    true
                }
            });

            // Re-render the bar so urgency (or the new focus) becomes visible.
            self.request_bar_redraw();

            if activated == Some(true) {
                log::debug!(
                    "xdg_activation: activated window {:?} (visible={}, app_id: {:?})",
                    win,
                    is_currently_visible,
                    token_data.app_id
                );
            } else {
                log::warn!(
                    "xdg_activation: failed to activate window {:?} (app_id: {:?})",
                    win,
                    token_data.app_id
                );
            }
            return;
        }

        if let Some(context) = launch_context {
            smithay::wayland::compositor::with_states(&surface, |states| {
                let _ = states.data_map.insert_if_missing_threadsafe(|| {
                    crate::backend::wayland::compositor::state::PendingLaunchContextMarker {
                        context,
                    }
                });
            });
            log::debug!(
                "xdg_activation: stored launch context for pending surface (app_id: {:?})",
                token_data.app_id
            );
        } else {
            log::warn!(
                "xdg_activation: missing launch context for pending surface (app_id: {:?})",
                token_data.app_id
            );
        }
    }
}
