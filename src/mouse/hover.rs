//! Hover-resize: cursor feedback and click-to-resize/move/close near floating
//! windows.
//!
//! When the pointer hovers just outside a floating window's border, the root
//! cursor changes to a resize shape.  A left-click then starts an interactive
//! resize (or move, when the cursor is at the window's top-middle edge);
//! a right-click always starts a move; a middle-click closes the window.
//! Moving further away deactivates the mode.
//!
//! The hover-offer *model* is shared; committing it to an interaction is a
//! backend concern (X11 commits through its modal grab loop in
//! `backend/x11/grab.rs`, Wayland through implicit pointer grabs).
//!
//! ## Entry points
//!
//! | Function                                      | Called from          | Purpose                                    |
//! |-----------------------------------------------|----------------------|--------------------------------------------|
//! | [`update_floating_resize_offer_at`]           | X11 motion           | Update resize offer + cursor, may focus    |
//! | [`update_any_floating_resize_offer_at`]       | Wayland motion       | Any-window offer + cursor, focus untouched |

use crate::contexts::WmCtx;
use crate::core_state::HoverOffer;
use crate::model::WmModel;
use crate::types::{AltCursor, Point, Rect, ResizeDirection, WindowId};

use super::constants::RESIZE_BORDER_ZONE;

// ── Hover offer helpers ──────────────────────────────────────────────────────
//
// Pure hover-offer state lives on [`crate::core_state::HoverOffer`] /
// [`crate::core_state::DragState`]; these functions apply the matching cursor.

/// Window and direction selected by the resize-border hit test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoverResizeTarget {
    pub win: WindowId,
    pub dir: ResizeDirection,
    pub geo: Rect,
}

/// Activate a resize hover offer and apply the matching cursor.
fn offer_hover_resize(ctx: &mut WmCtx, target: HoverResizeTarget) {
    ctx.core_mut()
        .state_mut()
        .drag
        .set_hover_offer(HoverOffer::Resize {
            win: target.win,
            dir: target.dir,
        });
    ctx.set_cursor_style(AltCursor::Resize(target.dir));
}

/// Clear any active hover offer and reset the cursor if the state changed.
pub fn clear_hover_offer(ctx: &mut WmCtx) {
    if ctx.core_mut().drag_state_mut().clear_hover_offer() {
        ctx.set_cursor_style(AltCursor::Default);
    }
}

fn resize_target_for_window(
    model: &WmModel,
    win: WindowId,
    root: Point,
) -> Option<HoverResizeTarget> {
    let view = model.client_view(win)?;
    let c = view.client;
    let mon = view.monitor;
    let selected_tags = mon.visible_tags();
    let has_tiling = mon.is_tiling_layout();

    if !c.is_visible(selected_tags) {
        return None;
    }
    if !c.mode().is_normal_floating() && has_tiling {
        return None;
    }
    if !c.geo.contains_resize_border_point(root, RESIZE_BORDER_ZONE) {
        return None;
    }

    let hit = c.geo.local_point(root);
    Some(HoverResizeTarget {
        win,
        dir: ResizeDirection::from_hit(c.geo.size(), hit),
        geo: c.geo,
    })
}

// ── Border detection ─────────────────────────────────────────────────────────

/// Return the floating window + direction currently targeted by hover-resize.
fn hover_resize_target_at(model: &WmModel, root: Point) -> Option<HoverResizeTarget> {
    let point = Rect::new(root.x, root.y, 1, 1);
    let monitor_id = model.monitors.id_intersecting_rect(point)?;
    let mon = model.monitor(monitor_id)?;
    if mon.bar_contains_y(&model.clients, root.y) {
        return None;
    }
    // Topmost first: the border the user *sees* must win the offer even when
    // focus order disagrees with the visible stacking. Stale ids are skipped
    // by the per-window visibility lookup.
    mon.z_order
        .iter_top_to_bottom()
        .find_map(|win| resize_target_for_window(model, win, root))
}

pub fn selected_hover_resize_target_at(
    model: &WmModel,
    position: Point,
) -> Option<HoverResizeTarget> {
    let win = model.selected_win()?;
    let monitor = model.client_view(win)?.monitor;
    if monitor.bar_contains_y(&model.clients, position.y) {
        return None;
    }
    resize_target_for_window(model, win, position)
}

/// Check whether any visible client on the current monitor is tiled.
fn has_visible_tiled_client(model: &WmModel) -> bool {
    let has_tiling = model.expect_selected_monitor().is_tiling_layout();
    let mon = model.expect_selected_monitor();
    let selected = mon.visible_tags();
    has_tiling
        && mon
            .iter_clients(&model.clients)
            .any(|(_, c)| c.is_visible(selected) && !c.mode().is_normal_floating())
}

// ── Motion-notify hook ───────────────────────────────────────────────────────

/// Updates the resize offer when the pointer is in a floating window border.
///
/// Returns `true` when the pointer is over a resize offer zone and the caller
/// should stop processing the motion event.
pub fn update_floating_resize_offer_at(ctx: &mut WmCtx, root: Point) -> bool {
    if let Some(target) = hover_resize_target_at(ctx.core().model(), root) {
        offer_hover_resize(ctx, target);
        // This function is only entered from physical X11 motion. The shared
        // mode policy still decides whether the resize offer may move focus.
        // Otherwise the motion handler resolves the actual window beneath the
        // pointer after the resize-offer check.
        let should_focus = ctx
            .core()
            .behavior()
            .focus_follows_mouse
            .allows(crate::types::HoverFocusTrigger::PointerMotion)
            && ctx.core().model().selected_win() != Some(target.win)
            && !has_visible_tiled_client(ctx.core().model());

        if should_focus {
            crate::focus::focus(ctx, Some(target.win));
        }
        return true;
    }

    clear_hover_offer(ctx);
    false
}

/// Update the resize offer scanning every visible floating window.
///
/// This is the Wayland motion path: hovering just outside *any* floating
/// window's border arms the resize offer and projects the matching cursor —
/// X11 parity for `update_floating_resize_offer_at`, minus its focus side
/// effects (Wayland decides hover focus separately).
///
/// Returns the window whose border is being offered, if any.
pub fn update_any_floating_resize_offer_at(ctx: &mut WmCtx, position: Point) -> Option<WindowId> {
    let Some(target) = hover_resize_target_at(ctx.core().model(), position) else {
        clear_hover_offer(ctx);
        return None;
    };
    offer_hover_resize(ctx, target);
    Some(target.win)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum SidebarOfferUpdate {
    None,
    Active,
    Cleared,
}

impl SidebarOfferUpdate {
    pub fn affects_pointer_handling(self) -> bool {
        !matches!(self, SidebarOfferUpdate::None)
    }
}

pub fn set_sidebar_offer(
    ctx: &mut WmCtx,
    target: Option<crate::types::SidebarTarget>,
) -> SidebarOfferUpdate {
    if let Some(target) = target {
        ctx.core_mut()
            .state_mut()
            .drag
            .set_hover_offer(HoverOffer::Sidebar(target));
        // Always project the cursor. Gesture completion can leave the same
        // logical offer in place while changing the active cursor override.
        ctx.set_cursor_style(AltCursor::VerticalAdjust);
        return SidebarOfferUpdate::Active;
    }

    if ctx.core().drag_state().hover_offer.is_sidebar() {
        clear_hover_offer(ctx);
        return SidebarOfferUpdate::Cleared;
    }

    SidebarOfferUpdate::None
}

pub fn update_sidebar_offer_at(
    ctx: &mut WmCtx,
    root: crate::types::Point,
    blocked_by_non_desktop: bool,
) -> SidebarOfferUpdate {
    let target = (!blocked_by_non_desktop)
        .then(|| crate::mouse::pointer::sidebar_target_at(ctx.core().model(), root))
        .flatten();
    set_sidebar_offer(ctx, target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::types::{Client, ClientMode, Monitor, TagMask, WindowId};
    use crate::wm::Wm;

    /// Two floating windows whose top border zones overlap: the visually
    /// topmost one must win the offer even when focus order puts the other
    /// window first.
    #[test]
    fn border_scan_prefers_the_topmost_window_over_focus_order() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let tags = TagMask::single(1).unwrap();
        let bottom = WindowId(1);
        let top = WindowId(2);

        let mut monitor = Monitor {
            monitor_rect: Rect::new(0, 0, 1920, 1080),
            show_bar: false,
            ..Monitor::default()
        };
        monitor.set_selected_tags(tags);
        // Focus order: `bottom` is the focused window.
        monitor.clients = vec![bottom, top];
        // Persistent z-order: `top` stacks above `bottom`.
        monitor.z_order.attach_top(bottom);
        monitor.z_order.attach_top(top);
        let monitor_id = wm.core.model.monitors.push(monitor);
        wm.core.model.monitors.set_selected(monitor_id);

        for (win, y) in [(bottom, 100), (top, 90)] {
            let mut client = Client {
                win,
                monitor_id,
                tags,
                geo: Rect::new(100, y, 600, 400),
                mode: ClientMode::floating(),
                ..Client::default()
            };
            client.set_placement(crate::types::ClientPlacement::Floating);
            wm.core.model.insert_client(client);
        }

        // Inside both windows' top border zones (30 px band above each edge).
        let target = hover_resize_target_at(&wm.core.model, Point::new(300, 85));
        assert_eq!(target.map(|target| target.win), Some(top));
    }

    #[test]
    fn border_scan_uses_the_monitor_under_the_pointer() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let tags = TagMask::single(1).unwrap();
        let left_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1920, 1080),
            ..Monitor::default()
        });
        let right_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(1920, 0, 1920, 1080),
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(left_id);
        wm.core
            .model
            .monitor_mut(right_id)
            .unwrap()
            .set_selected_tags(tags);

        let win = WindowId(3);
        let mut client = Client {
            win,
            monitor_id: right_id,
            tags,
            geo: Rect::new(2000, 100, 600, 400),
            mode: ClientMode::floating(),
            ..Client::default()
        };
        client.set_placement(crate::types::ClientPlacement::Floating);
        wm.core.model.insert_client(client);
        let right = wm.core.model.monitor_mut(right_id).unwrap();
        right.clients.push(win);
        right.z_order.attach_top(win);

        let target = hover_resize_target_at(&wm.core.model, Point::new(2200, 95));
        assert_eq!(target.map(|target| target.win), Some(win));
        assert_eq!(wm.core.model.selected_monitor_id(), left_id);
    }
}
