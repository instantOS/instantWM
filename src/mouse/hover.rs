//! Hover-resize: cursor feedback and click-to-resize around floating windows
//! and in gaps between tiled windows.
//!
//! When the pointer hovers just outside a floating window's border, the root
//! cursor changes to a resize shape.  A left-click then starts an interactive
//! resize (or move, when the cursor is at the window's top-middle edge);
//! a right-click always starts a move; a middle-click closes the window.
//! Moving further away deactivates the mode.
//! The offer is occlusion-aware: a border seam hidden beneath another
//! window's surface is never offered, and the scan stops at the first
//! surface the pointer is actually over.
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
use crate::model::{ClientView, WmModel};
use crate::types::{Monitor, Point, Rect, ResizeDirection, WindowId};

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

fn resize_target_for_window(view: ClientView<'_>, root: Point) -> Option<HoverResizeHit> {
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
        win: c.win,
        dir: ResizeDirection::from_hit(c.geo.size(), hit),
        geo: c.geo,
    })
}

// ── Border detection ─────────────────────────────────────────────────────────

/// `true` when a visible window's surface — borders included — covers `point`.
///
/// Mirrors the backend hit tests: what matters is the surface the pointer is
/// actually over, not focus order. Hidden windows do not occlude.
fn view_covers_point(view: ClientView<'_>, point: Point) -> bool {
    view.client.is_visible(view.monitor.visible_tags())
        && view.client.total_rect().contains_point(point)
}

/// [`view_covers_point`] by window id. Stale ids simply do not occlude.
fn is_point_over_client_surface(model: &WmModel, win: WindowId, point: Point) -> bool {
    model
        .client_view(win)
        .is_some_and(|view| view_covers_point(view, point))
}

/// `true` when a visible window stacked above `win` covers `point`.
fn point_occluded_above(model: &WmModel, monitor: &Monitor, win: WindowId, point: Point) -> bool {
    monitor
        .z_order
        .iter_top_to_bottom()
        .take_while(|&above| above != win)
        .any(|above| is_point_over_client_surface(model, above, point))
}

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
    // by the per-window visibility lookup. A window whose surface covers the
    // pointer equally hides every border below it, so the scan stops there
    // rather than offering the seam of a covered window. One resolved view
    // per window serves both the band check and the occlusion stop.
    for win in mon.z_order.iter_top_to_bottom() {
        let Some(view) = model.client_view(win) else {
            continue;
        };
        if let Some(hit) = resize_target_for_window(view, root) {
            return Some(hit);
        }
        if view_covers_point(view, root) {
            return None;
        }
    }
    None
}

pub fn selected_hover_resize_target_at(model: &WmModel, position: Point) -> Option<HoverResizeHit> {
    let win = model.selected_win()?;
    let view = model.client_view(win)?;
    if view.monitor.bar_contains_y(&model.clients, position.y) {
        return None;
    }
    // A click must never commit a border the user cannot see: when another
    // window's surface covers the position, the press belongs to that window.
    // Checked before the band test so an occluded seam skips the hit math.
    if point_occluded_above(model, view.monitor, win, position) {
        return None;
    }
    resize_target_for_window(view, position)
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

    /// Insert two floating windows with `bottom` focused but stacked beneath
    /// `top`, mirroring the focus/stacking split of the overlap test above.
    fn setup_stacked_floating_windows(wm: &mut Wm, bottom_geo: Rect, top_geo: Rect) {
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
        monitor.selected = Some(bottom);
        // Persistent z-order: `top` stacks above `bottom`.
        monitor.z_order.attach_top(bottom);
        monitor.z_order.attach_top(top);
        let monitor_id = wm.core.model.monitors.push(monitor);
        wm.core.model.monitors.set_selected(monitor_id);

        for (win, geo) in [(bottom, bottom_geo), (top, top_geo)] {
            let mut client = Client {
                win,
                monitor_id,
                tags,
                geo,
                mode: ClientMode::floating(),
                ..Client::default()
            };
            client.set_placement(crate::types::ClientPlacement::Floating);
            wm.core.model.insert_client(client);
        }
    }

    /// A smaller floating window fully covered by a larger one must not offer
    /// its border seams through the covering window's surface: neither the
    /// passive motion offer nor the selected-window click commit may see them.
    #[test]
    fn covered_window_borders_are_not_offered_through_the_covering_surface() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        setup_stacked_floating_windows(
            &mut wm,
            Rect::new(150, 130, 400, 300),
            Rect::new(100, 90, 600, 400),
        );

        // Inside the covered window's top border zone (y 100..130) but
        // strictly inside the covering window's surface.
        assert_eq!(
            hover_resize_target_at(&wm.core.model, Point::new(300, 110)),
            None
        );
        assert_eq!(
            selected_hover_resize_target_at(&wm.core.model, Point::new(300, 110)),
            None
        );

        // The covering window's own border still offers when hovered directly.
        let target = hover_resize_target_at(&wm.core.model, Point::new(300, 85));
        assert_eq!(target.map(|hit| hit.win), Some(WindowId(2)));
    }

    /// Occlusion must only suppress the hidden part of a seam: a covered
    /// window whose border pokes out beyond the covering window is still
    /// offered on its exposed side.
    #[test]
    fn exposed_border_of_a_covered_window_still_offers() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        setup_stacked_floating_windows(
            &mut wm,
            Rect::new(750, 130, 400, 300),
            Rect::new(100, 90, 600, 400),
        );

        // In the covered window's left border zone, outside the cover's
        // surface (the cover's band ends at x = 700 + 30).
        let target = hover_resize_target_at(&wm.core.model, Point::new(735, 200));
        assert_eq!(target.map(|hit| hit.win), Some(WindowId(1)));
        assert_eq!(
            selected_hover_resize_target_at(&wm.core.model, Point::new(735, 200))
                .map(|hit| hit.win),
            Some(WindowId(1))
        );
    }
}
