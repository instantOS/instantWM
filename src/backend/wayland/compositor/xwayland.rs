use smithay::{
    utils::SERIAL_COUNTER,
    wayland::selection::SelectionTarget,
    xwayland::{X11Surface, XwmHandler, xwm::WmWindowProperty},
};
use std::os::unix::io::OwnedFd;

use crate::client::FloatingPlacementKind;
use crate::types::{ClientMode, MouseButton, Point, ResizeDirection, WindowId};

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
    let pointer = state.pointer.current_location();
    let root = Point::new(pointer.x.round() as i32, pointer.y.round() as i32);
    let Some(g) = state.globals_mut() else {
        return;
    };
    if g.drag.interactive.active {
        return;
    }
    let Some(client) = g.clients.get(&win) else {
        return;
    };
    if !client.mode.is_floating() {
        return;
    }
    let geo = client.geo;
    let sel = g.selected_win();
    let was_hidden = client.is_hidden;
    g.drag.interactive = crate::globals::DragInteraction {
        active: true,
        win,
        button: MouseButton::Left,
        dragging: false,
        drag_type: crate::globals::DragType::Move,
        was_focused: sel == Some(win),
        was_hidden,
        start_point: root,
        win_start_geo: geo,
        drop_restore_geo: geo,
        last_root_point: root,
        suppress_click_action: true,
    };
}

/// Begin an interactive resize drag triggered by a client request
/// (`_NET_WM_MOVERESIZE` resize on X11, `xdg_toplevel.resize` on Wayland).
///
/// Unlike app-initiated move, the user is already actively grabbing a
/// resize handle, so this skips the click-vs-drag threshold and engages
/// the resize immediately (`dragging = true`). The unified
/// [`crate::wayland::input::pointer::drag::wayland_hover_resize_drag_motion`]
/// handler then drives the resize from subsequent pointer motion events.
pub(super) fn begin_app_resize_drag(
    state: &mut WaylandState,
    win: WindowId,
    dir: ResizeDirection,
) {
    state.activate_and_raise_window(win);
    let pointer = state.pointer.current_location();
    let root = Point::new(pointer.x.round() as i32, pointer.y.round() as i32);
    let started = {
        let Some(g) = state.globals_mut() else {
            return;
        };
        if g.drag.interactive.active {
            return;
        }
        let Some(client) = g.clients.get(&win) else {
            return;
        };
        if !client.mode.is_floating() {
            return;
        }
        let geo = client.geo;
        let sel = g.selected_win();
        let was_hidden = client.is_hidden;
        g.drag.interactive = crate::globals::DragInteraction {
            active: true,
            win,
            button: MouseButton::Left,
            dragging: true,
            drag_type: crate::globals::DragType::Resize(dir),
            was_focused: sel == Some(win),
            was_hidden,
            start_point: root,
            win_start_geo: geo,
            drop_restore_geo: geo,
            last_root_point: root,
            suppress_click_action: true,
        };
        true
    };
    if started {
        state.begin_interactive_resize(win);
        state.with_wm_mut_unified(|wm, _state| {
            let mut ctx = wm.ctx();
            crate::mouse::set_cursor_style(&mut ctx, crate::types::AltCursor::Resize(dir));
        });
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
) -> Option<crate::types::WindowId> {
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

        if (transient_parent.is_some()
            || client.is_fixed_size
            || should_float_for_type
            || surface.is_above())
            && !client.mode.is_floating()
        {
            client.float_geo = client.geo;
            client.mode = ClientMode::Floating;
            changed_layout = true;
        }

        if surface.is_fullscreen() {
            client.mode = client.mode.as_fullscreen();
        } else if client.mode.is_fullscreen() {
            client.mode = client.mode.restored();
        }
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
    transient_parent
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
            let parent = apply_xwayland_surface_policy(self, win, &window);
            self.map_window(win);
            self.activate_and_raise_window(win);
            if let Some(rect) = self.globals().and_then(|g| {
                g.clients
                    .get(&win)
                    .filter(|client| client.mode.is_floating())
                    .map(|c| c.geo)
            }) {
                let resolved = self.globals_mut().map(|g| {
                    crate::client::resolve_and_sync_floating_geometry(
                        g,
                        win,
                        rect,
                        FloatingPlacementKind::New,
                        parent,
                    )
                });
                if let Some(resolved) = resolved {
                    self.resize_window(win, resolved);
                }
            }
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
        let parent = apply_xwayland_surface_policy(self, win, &window);
        if let Some(g) = self.globals_mut() {
            crate::client::resolve_and_sync_floating_geometry(
                g,
                win,
                crate::types::Rect {
                    x: geo.loc.x,
                    y: geo.loc.y,
                    w: geo.size.w.max(1),
                    h: geo.size.h.max(1),
                },
                FloatingPlacementKind::New,
                parent,
            );
        }
        let final_rect = if let Some(rect) = self
            .globals()
            .and_then(|g| crate::client::sane_floating_spawn_rect(g, win, parent))
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
        if let Some(win) = self.window_id_for_x11_surface(&window)
            && let Some(resolved) = self.globals_mut().map(|g| {
                crate::client::resolve_and_sync_floating_geometry(
                    g,
                    win,
                    crate::types::Rect {
                        x: geo.loc.x,
                        y: geo.loc.y,
                        w: geo.size.w.max(1),
                        h: geo.size.h.max(1),
                    },
                    FloatingPlacementKind::AppRequest,
                    None,
                )
            })
        {
            geo.loc.x = resolved.x;
            geo.loc.y = resolved.y;
            geo.size.w = resolved.w;
            geo.size.h = resolved.h;
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
        let rect = if let Some(g) = self.globals_mut() {
            crate::client::resolve_and_sync_floating_geometry(
                g,
                win,
                crate::types::Rect {
                    x: geometry.loc.x,
                    y: geometry.loc.y,
                    w: geometry.size.w.max(1),
                    h: geometry.size.h.max(1),
                },
                FloatingPlacementKind::BackendObserved,
                None,
            )
        } else {
            crate::types::Rect {
                x: geometry.loc.x,
                y: geometry.loc.y,
                w: geometry.size.w.max(1),
                h: geometry.size.h.max(1),
            }
        };
        // This is an acknowledgement/notification from XWayland about the
        // geometry it is already using. Feeding it back into `resize_window`
        // would send another X11 configure and create a resize loop.
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
                crate::client::mode::set_maximized(ctx_wayland.core.globals_mut(), win, true);
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
                let outcome =
                    crate::client::mode::set_maximized(ctx_wayland.core.globals_mut(), win, false);
                if let (
                    Some(crate::client::mode::MaximizedOutcome::Exited { .. }),
                    Some(restore_rect),
                ) = (outcome, restore_rect)
                {
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
            crate::client::mode::set_fullscreen(g, win, true);
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
            crate::client::mode::set_fullscreen(g, win, false);
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
