//! Hover-resize: cursor feedback and click-to-resize around floating windows
//! and in gaps between tiled windows.
//!
//! When the pointer hovers just outside a floating window's border, the root
//! cursor changes to a resize shape.  A left-click then starts an interactive
//! resize (or move, when the cursor is at the window's top-middle edge);
//! a right-click always starts a move; a middle-click closes the window.
//! Moving further away deactivates the mode.
//! An adjustable inner tiling gap instead offers the corresponding tree seam;
//! dragging either primary button there has the same layout effect as a
//! Super+right-button resize on the adjacent tile.
//!
//! The hover-offer model is authoritative: cursor presentation and pointer
//! routing are derived from it and reconciled by the active backend. Committing
//! an offer remains a transport concern (X11 uses its modal interaction loop;
//! Wayland uses compositor-owned pointer input).
//!
//! ## Entry points
//!
//! | Function                                      | Called from          | Purpose                                    |
//! |-----------------------------------------------|----------------------|--------------------------------------------|
//! | [`update_resize_offer_with_focus_at`]         | X11 motion           | Update resize offer + cursor, may focus    |
//! | [`update_resize_offer_at`]                    | Wayland motion       | Update resize offer + cursor only          |

use crate::contexts::WmCtx;
use crate::core_state::HoverOffer;
use crate::model::WmModel;
use crate::types::{Point, Rect, ResizeDirection, WindowId};

use super::constants::RESIZE_BORDER_ZONE;

// ── Hover offer helpers ──────────────────────────────────────────────────────
//
// Pure hover-offer state lives on [`crate::core_state::HoverOffer`] /
// [`crate::core_state::PointerInteractionState`]; these functions reconcile its derived
// presentation after each transition.

/// Window and direction selected by the resize-border hit test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoverResizeHit {
    pub win: WindowId,
    pub dir: ResizeDirection,
    pub geo: Rect,
}

/// Activate a resize hover offer and reconcile its derived presentation.
fn offer_hover_resize(ctx: &mut WmCtx, target: HoverResizeHit) {
    let changed = ctx.transition_pointer_interaction(|drag| {
        drag.set_hover_offer(HoverOffer::Resize {
            win: target.win,
            dir: target.dir,
        })
    });
    if !changed {
        // Retry a previously failed native projection while the offer remains
        // authoritative (for example, an X11 grab becoming available).
        ctx.sync_interaction_projection();
    }
}

fn offer_tree_resize(ctx: &mut WmCtx, win: WindowId, direction: ResizeDirection) {
    let changed = ctx.transition_pointer_interaction(|drag| {
        drag.set_hover_offer(HoverOffer::TreeResize {
            win,
            dir: direction,
        })
    });
    if !changed {
        ctx.sync_interaction_projection();
    }
}

/// Update the passive resize offer at a pointer position without applying
/// hover-focus policy. Returns the window owning the offered resize seam.
pub fn update_resize_offer_at(ctx: &mut WmCtx, root: Point) -> Option<WindowId> {
    if let Some(target) = hover_resize_target_at(ctx.core().model(), root) {
        offer_hover_resize(ctx, target);
        return Some(target.win);
    }
    if let Some((win, resize)) = crate::layouts::manager::pointer_tree_gap_resize_start(ctx, root) {
        offer_tree_resize(ctx, win, resize.direction);
        return Some(win);
    }
    clear_hover_offer(ctx);
    None
}

/// Clear any active hover offer and reconcile the resulting presentation.
pub fn clear_hover_offer(ctx: &mut crate::contexts::WmCtx) {
    // Reconciliation is intentionally unconditional so native state can heal
    // even after a redundant logical clear.
    let changed = ctx.transition_pointer_interaction(|drag| drag.clear_hover_offer());
    if !changed {
        ctx.sync_interaction_projection();
    }
}

fn resize_target_for_window(model: &WmModel, win: WindowId, root: Point) -> Option<HoverResizeHit> {
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
    Some(HoverResizeHit {
        win,
        dir: ResizeDirection::from_hit(c.geo.size(), hit),
        geo: c.geo,
    })
}

// ── Border detection ─────────────────────────────────────────────────────────

/// Return the floating window + direction currently targeted by hover-resize.
fn hover_resize_target_at(model: &WmModel, root: Point) -> Option<HoverResizeHit> {
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

pub fn selected_hover_resize_target_at(model: &WmModel, position: Point) -> Option<HoverResizeHit> {
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

/// Updates the resize offer at floating borders and adjustable inner gaps.
///
/// Returns `true` when the pointer is over a resize offer zone and the caller
/// should stop processing the motion event.
pub fn update_resize_offer_with_focus_at(ctx: &mut WmCtx, root: Point) -> bool {
    if let Some(win) = update_resize_offer_at(ctx, root) {
        // This function is only entered from physical X11 motion. The shared
        // mode policy still decides whether the resize offer may move focus.
        // Otherwise the motion handler resolves the actual window beneath the
        // pointer after the resize-offer check.
        let should_focus = ctx
            .core()
            .behavior()
            .focus_follows_mouse
            .allows(crate::types::HoverFocusTrigger::PointerMotion)
            && ctx.core().model().selected_win() != Some(win)
            && !has_visible_tiled_client(ctx.core().model());

        if should_focus {
            crate::focus::focus(ctx, Some(win));
        }
        return true;
    }
    false
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
        // Always reconcile. Gesture completion can leave the same logical
        // offer in place while ending a higher-priority captured interaction.
        let changed = ctx.transition_pointer_interaction(|drag| {
            drag.set_hover_offer(HoverOffer::Sidebar(target))
        });
        if !changed {
            ctx.sync_interaction_projection();
        }
        return SidebarOfferUpdate::Active;
    }

    if ctx.core().interaction().drag.hover_offer().is_sidebar() {
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
