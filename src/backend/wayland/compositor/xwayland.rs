use smithay::{
    utils::SERIAL_COUNTER,
    wayland::selection::SelectionTarget,
    xwayland::{X11Surface, XwmHandler, xwm::WmWindowProperty},
};
use std::os::unix::io::OwnedFd;

use crate::types::{ResizeDirection, WindowId};

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

/// Map a Smithay XWayland resize edge to a [`ResizeDirection`].
pub(super) fn xwayland_resize_edge_to_direction(
    edge: smithay::xwayland::xwm::ResizeEdge,
) -> ResizeDirection {
    use smithay::xwayland::xwm::ResizeEdge as E;
    match edge {
        E::Top => ResizeDirection::Top,
        E::Bottom => ResizeDirection::Bottom,
        E::Left => ResizeDirection::Left,
        E::Right => ResizeDirection::Right,
        E::TopLeft => ResizeDirection::TopLeft,
        E::TopRight => ResizeDirection::TopRight,
        E::BottomLeft => ResizeDirection::BottomLeft,
        E::BottomRight => ResizeDirection::BottomRight,
    }
}

/// Map an `xdg_toplevel` resize edge to a [`ResizeDirection`].
///
/// Returns `None` for `xdg_toplevel::ResizeEdge::None` or any unknown
/// future variant.
pub(super) fn xdg_resize_edge_to_direction(
    edge: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
) -> Option<ResizeDirection> {
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge as E;
    Some(match edge {
        E::Top => ResizeDirection::Top,
        E::Bottom => ResizeDirection::Bottom,
        E::Left => ResizeDirection::Left,
        E::Right => ResizeDirection::Right,
        E::TopLeft => ResizeDirection::TopLeft,
        E::TopRight => ResizeDirection::TopRight,
        E::BottomLeft => ResizeDirection::BottomLeft,
        E::BottomRight => ResizeDirection::BottomRight,
        _ => return None,
    })
}

/// Begin an interactive move drag triggered by a client request
/// (`_NET_WM_MOVERESIZE` move on X11, `xdg_toplevel.move` on Wayland).
///
/// The drag is started in the same "pre-threshold" state as a title bar
/// click: `active = true`, `dragging = false`. The next pointer motion
/// event will exceed the threshold and promote it to an actual drag via
/// the existing title-drag motion handler.
pub(super) fn begin_app_move_drag(state: &mut WaylandState, win: WindowId) {
    state.activate_and_raise_window(win);
    state.push_command(super::super::commands::WmCommand::BeginMove(win));
}

/// Begin an interactive resize drag triggered by a client request
/// (`_NET_WM_MOVERESIZE` resize on X11, `xdg_toplevel.resize` on Wayland).
///
/// Unlike app-initiated move, the user is already actively grabbing a
/// resize handle, so this skips the click-vs-drag threshold and engages
/// the resize immediately (`dragging = true`). The unified
/// [`crate::wayland::input::pointer::drag::wayland_hover_resize_drag_motion`]
/// handler then drives the resize from subsequent pointer motion events.
pub(super) fn begin_app_resize_drag(state: &mut WaylandState, win: WindowId, dir: ResizeDirection) {
    state.activate_and_raise_window(win);
    state.push_command(super::super::commands::WmCommand::BeginResize { win, dir });
}

/// Trigger a pointer focus update to ensure hover state is correct.
pub(super) fn trigger_pointer_focus_update(state: &mut WaylandState) {
    state.push_command(super::super::commands::WmCommand::PointerMotion { time_msec: 0 });
}

fn sync_xwayland_surface_metadata(
    state: &mut WaylandState,
    win: crate::types::WindowId,
    surface: &X11Surface,
) {
    let properties = crate::client::x11_policy::window_properties_from_x11_surface(surface);
    state.push_command(super::super::commands::WmCommand::UpdateProperties { win, properties });
    state.update_foreign_toplevel(win);
    state.request_bar_redraw();
}

fn apply_xwayland_surface_policy(
    state: &mut WaylandState,
    win: crate::types::WindowId,
    surface: &X11Surface,
) {
    state.push_command(super::super::commands::WmCommand::UpdateXWaylandPolicy {
        win,
        hints: surface.hints(),
        size_hints: surface.size_hints(),
        is_fullscreen: surface.is_fullscreen(),
        is_hidden: surface.is_hidden(),
        is_above: surface.is_above(),
    });
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
        let geo = window.geometry();

        if is_unmanaged_x11_overlay(&window) {
            let window_id = window.window_id();
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

        let parent = crate::client::x11_policy::transient_for_window_id(&window)
            .and_then(|parent_x11| self.window_id_for_x11_window(parent_x11.into()));

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

        self.space.map_element(element.clone(), geo.loc, false);
        self.window_index.insert(win, element);

        let properties = self.window_properties(win);
        let initial_geo = Some(crate::types::Rect {
            x: geo.loc.x,
            y: geo.loc.y,
            w: geo.size.w.max(1),
            h: geo.size.h.max(1),
        });

        self.push_command(super::super::commands::WmCommand::MapWindow(
            super::super::commands::MapWindowParams {
                win,
                properties,
                initial_geo,
                launch_pid: window.pid(),
                launch_startup_id: window.startup_id(),
                x11_hints: window.hints(),
                x11_size_hints: window.size_hints(),
                parent,
            },
        ));

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
            self.push_command(super::super::commands::WmCommand::UnmanageWindow(win));
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
        if let Some(win) = self.window_id_for_x11_surface(&window) {
            self.push_command(super::super::commands::WmCommand::UpdateWindowSize {
                win,
                w: geo.size.w,
                h: geo.size.h,
            });
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

        self.push_command(super::super::commands::WmCommand::UpdateWindowSize {
            win,
            w: geometry.size.w,
            h: geometry.size.h,
        });

        // This is an acknowledgement/notification from XWayland about the
        // geometry it is already using. Feeding it back into `resize_window`
        // would send another X11 configure and create a resize loop.
        let rect = crate::types::Rect::new(
            geometry.loc.x,
            geometry.loc.y,
            geometry.size.w,
            geometry.size.h,
        );
        let mode = self.default_window_move_mode();
        self.set_window_target_rect(win, rect, mode);
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
                    self.push_command(super::super::commands::WmCommand::RequestSpaceSync);
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
        self.push_command(super::super::commands::WmCommand::SetMaximized {
            win,
            maximized: true,
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
        self.push_command(super::super::commands::WmCommand::SetMaximized {
            win,
            maximized: false,
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
        self.push_command(super::super::commands::WmCommand::SetFullscreen {
            win,
            fullscreen: true,
        });
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
        self.push_command(super::super::commands::WmCommand::SetFullscreen {
            win,
            fullscreen: false,
        });
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
        self.push_command(super::super::commands::WmCommand::SetMinimized {
            win,
            minimized: true,
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
        self.push_command(super::super::commands::WmCommand::SetMinimized {
            win,
            minimized: false,
        });
    }

    fn resize_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
        _button: u32,
        resize_edge: smithay::xwayland::xwm::ResizeEdge,
    ) {
        let Some(win) = self.window_id_for_x11_surface(&window) else {
            return;
        };
        let dir = xwayland_resize_edge_to_direction(resize_edge);
        begin_app_resize_drag(self, win, dir);
    }

    fn move_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
        _button: u32,
    ) {
        let Some(win) = self.window_id_for_x11_surface(&window) else {
            return;
        };
        begin_app_move_drag(self, win);
    }

    fn active_window_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::X11Surface,
        _serial: u32,
        _parent: Option<smithay::xwayland::X11Surface>,
    ) {
        if let Some(win) = self.window_id_for_x11_surface(&window) {
            self.activate_and_raise_window(win);
        }
    }

    fn allow_selection_access(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _selection: SelectionTarget,
    ) -> bool {
        true
    }

    fn send_selection(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        use smithay::wayland::selection::data_device::request_data_device_client_selection;
        use smithay::wayland::selection::primary_selection::request_primary_client_selection;
        let seat = self.seat.clone();
        match selection {
            SelectionTarget::Clipboard => {
                if let Err(err) = request_data_device_client_selection(&seat, mime_type, fd) {
                    log::warn!(
                        "Failed to request current wayland clipboard for XWayland: {:?}",
                        err
                    );
                }
            }
            SelectionTarget::Primary => {
                if let Err(err) = request_primary_client_selection(&seat, mime_type, fd) {
                    log::warn!(
                        "Failed to request current wayland primary selection for XWayland: {:?}",
                        err
                    );
                }
            }
        }
    }

    fn new_selection(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        selection: SelectionTarget,
        mime_types: Vec<String>,
    ) {
        use smithay::wayland::selection::data_device::set_data_device_selection;
        use smithay::wayland::selection::primary_selection::set_primary_selection;
        let seat = self.seat.clone();
        match selection {
            SelectionTarget::Clipboard => {
                set_data_device_selection(&self.display_handle, &seat, mime_types, ());
            }
            SelectionTarget::Primary => {
                set_primary_selection(&self.display_handle, &seat, mime_types, ());
            }
        }
    }

    fn cleared_selection(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        selection: SelectionTarget,
    ) {
        use smithay::wayland::selection::data_device::{
            clear_data_device_selection, current_data_device_selection_userdata,
        };
        use smithay::wayland::selection::primary_selection::{
            clear_primary_selection, current_primary_selection_userdata,
        };
        let seat = self.seat.clone();
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&seat).is_some() {
                    clear_data_device_selection(&self.display_handle, &seat);
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&seat).is_some() {
                    clear_primary_selection(&self.display_handle, &seat);
                }
            }
        }
    }

    fn disconnected(&mut self, _xwm: smithay::xwayland::xwm::XwmId) {
        self.xwm = None;
        self.xdisplay = None;
    }
}
