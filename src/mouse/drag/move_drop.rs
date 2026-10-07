//! Move and drop operations for window dragging.
//!
//! This module contains the core logic for moving windows with the mouse,
//! including bar hover handling, edge snapping, and drop completion.
use crate::backend::PointerOps;
use crate::layouts::ArrangeAnimation;

use crate::client::geometry::FloatingPlacementIntent;
use crate::contexts::WmCtx;
use crate::core_state::CoreState;
use crate::floating::{WindowModeRequest, set_window_mode};
use crate::layouts::PresentationMode;
use crate::layouts::arrange;
use crate::types::*;

use crate::mouse::constants::OVERLAY_ZONE_WIDTH;

use crate::monitor::{TransferFocus, transfer_client};

/// Snap `position` to the edges of the work area under `root`.
///
/// `border_width` is the dragged client's modelled border, resolved by the
/// caller from the client it already holds; an unmanaged drag target counts
/// as `0`.
pub fn snap_window_to_monitor_edges(
    state: &CoreState,
    border_width: i32,
    content_size: Size,
    position: &mut Point,
    root: Point,
) {
    let snap = state.config.window.snap_threshold;
    let Some(monitor) = state
        .model
        .monitors
        .monitor_intersecting_rect(Rect::new(root.x, root.y, 1, 1))
    else {
        return;
    };
    let outer_size = Size::new(
        content_size.w + border_width * 2,
        content_size.h + border_width * 2,
    );
    let work_rect = monitor.work_rect();

    if (work_rect.x - position.x).abs() < snap {
        position.x = work_rect.x;
    } else if (work_rect.right() - (position.x + outer_size.w)).abs() < snap {
        position.x = work_rect.right() - outer_size.w;
    }

    if (work_rect.y - position.y).abs() < snap {
        position.y = work_rect.y;
    } else if (work_rect.bottom() - (position.y + outer_size.h)).abs() < snap {
        position.y = work_rect.bottom() - outer_size.h;
    }
}

/// A move destination is resolved from pointer geometry, independently of focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MoveDropTarget {
    Bar(MonitorId),
    Tree(MonitorId),
    Snap(MonitorId, SnapPosition),
    Free(MonitorId),
}

impl MoveDropTarget {
    fn monitor(self) -> MonitorId {
        match self {
            Self::Bar(id) | Self::Tree(id) | Self::Snap(id, _) | Self::Free(id) => id,
        }
    }
}

/// Resolve the destination of a move whose dragged `client` is already known.
///
/// The client's mode decides between tree, edge snap, and free placement, so
/// callers pass the view they resolved for this sample instead of paying for
/// a second lookup here.
pub(crate) fn resolve_move_drop(
    model: &crate::model::WmModel,
    client: &Client,
    root: Point,
) -> Option<MoveDropTarget> {
    let mon = model
        .monitors
        .monitor_intersecting_rect(Rect::new(root.x, root.y, 1, 1))?;
    if bar_hovered(mon, root) {
        return Some(MoveDropTarget::Bar(mon.id()));
    }
    if client.mode().is_normal_tiling() && mon.current_layout() == PresentationMode::Tiled {
        return Some(MoveDropTarget::Tree(mon.id()));
    }
    // Tiled sources keep their placement when entering a different presentation.
    // Floating windows retain directional edge snapping on floating outputs.
    if mon.current_layout() == PresentationMode::Floating && client.mode().is_normal_floating() {
        let edge = if root.x < mon.monitor_rect.x + OVERLAY_ZONE_WIDTH {
            Some(SnapPosition::Left)
        } else if root.x >= mon.monitor_rect.right() - OVERLAY_ZONE_WIDTH {
            Some(SnapPosition::Right)
        } else {
            None
        };
        if let Some(edge) = edge {
            return Some(MoveDropTarget::Snap(mon.id(), edge));
        }
    }
    Some(MoveDropTarget::Free(mon.id()))
}

/// Whether `root` sits on `monitor`'s visible bar.
fn bar_hovered(monitor: &Monitor, root: Point) -> bool {
    monitor.bar_visible()
        && monitor.y_in_bar(root.y)
        && root.x >= monitor.monitor_rect.x
        && root.x < monitor.monitor_rect.right()
}

fn bar_monitor_at(model: &crate::model::WmModel, root: Point) -> Option<&crate::types::Monitor> {
    let monitor = model
        .monitors
        .monitor_intersecting_rect(Rect::new(root.x, root.y, 1, 1))?;
    bar_hovered(monitor, root).then_some(monitor)
}

// ── move_mouse helpers ────────────────────────────────────────────────────

/// Set the drag hover and gesture highlight when the cursor enters the bar,
/// and clears them when it leaves.  Returns `true` while on the bar.
pub fn update_bar_hover_simple(ctx: &mut WmCtx, root: Point) -> bool {
    let bar_hit = bar_monitor_at(&ctx.core().state.model, root).map(|monitor| {
        let core = ctx.core();
        let gesture =
            crate::bar::model::bar_position_at_x(monitor, core, monitor.local_work_point(root).x)
                .to_gesture();
        (monitor.id(), gesture)
    });
    let on_bar = bar_hit.is_some();
    let was_on_bar = ctx.core().bar.hover.drag_active;

    if let Some((monitor_id, new_gesture)) = bar_hit {
        if ctx.core_mut().bar.hover.set(monitor_id, new_gesture, true) {
            ctx.request_bar_update();
        }
    } else if was_on_bar {
        ctx.core_mut().bar.hover.clear();
        ctx.request_bar_update();
    }

    on_bar
}

/// Clears `bar_dragging` and redraws the bar unconditionally.
///
/// Called once the drag loop exits so that hover state is always cleaned up.
pub fn clear_bar_hover(ctx: &mut WmCtx) {
    ctx.core_mut().bar.hover.clear();
    ctx.request_bar_update();
}

/// Handle a drop onto the bar: tile the window, optionally moving it to the
/// hovered tag first.
///
/// Mirrors the C `handle_bar_drop`:
/// * Dropped on a tag button → `set_window_mode(Tiled)` + `tag()`
/// * Dropped elsewhere on bar, window floating → `set_window_mode(Tiled)`
///
/// # `grab_start_rect`
///
/// The window geometry at the moment the drag started.  When the window was
/// floating, this is the true pre-drag origin; we preserve its size in the
/// saved floating placement
/// so un-tiling later restores the original floating position.
pub fn handle_bar_drop(
    ctx: &mut WmCtx,
    win: WindowId,
    grab_start_rect: Rect,
    pointer_override: Option<Point>,
    modifiers: ModMask,
) {
    let Some(root) = pointer_override.or_else(|| ctx.pointer_location()) else {
        return;
    };
    let Some(mon) = bar_monitor_at(&ctx.core().state.model, root) else {
        return;
    };
    let monitor_id = mon.id();
    let position =
        crate::bar::model::bar_position_at_x(mon, ctx.core(), mon.local_work_point(root).x);
    let focus = if matches!(position, BarPosition::Tag(_)) {
        TransferFocus::Preserve
    } else {
        TransferFocus::FollowWindow
    };
    let already_on_bar_monitor = mon.has_client(win);
    if !already_on_bar_monitor && transfer_client(ctx, win, monitor_id, focus).is_none() {
        return;
    }

    // Remember whether the window was floating *before* any state change so
    // we know whether to correct the saved floating placement afterwards.
    // The tag/tile actions below both only read what is captured here, so
    // this is the single resolution after the transfer.
    let (was_floating, is_true_fullscreen) = match ctx.core().state.model.client(win) {
        Some(c) => (
            c.placement() == ClientPlacement::Floating,
            c.mode().is_true_fullscreen(),
        ),
        None => return,
    };

    if let BarPosition::Tag(tag_idx) = position {
        // Tile first (no arrange), then tag.
        //
        // Old order: tag() → arrange(, ArrangeAnimation::Configured) [window still floating, layout skips
        // it] → set_window_mode() → arrange(, ArrangeAnimation::Configured) again.  That's two arrange passes.
        //
        // New order: set_window_mode saves the floating placement from the
        // current floating geometry *before* tag() calls arrange(, ArrangeAnimation::Configured).  Then
        // tag() calls arrange(, ArrangeAnimation::Configured) exactly once with the window already marked
        // tiled, so the layout places it correctly in a single pass.
        //

        // Don't tile fullscreen windows
        if !is_true_fullscreen {
            let _ = set_window_mode(ctx, win, WindowModeRequest::Tiling);
        }
        crate::mouse::drag::tag::apply_window_tag_drop(
            ctx,
            win,
            TagMask::from_index(tag_idx).unwrap_or(TagMask::EMPTY),
            modifiers,
        );
    } else if was_floating {
        // Dropped on the bar but not on a tag button: tile the window.
        // Use set_window_mode directly instead of toggle_floating() which
        // operates on mon.sel — a value that could theoretically diverge from
        // the window we actually dragged.
        let _ = set_window_mode(ctx, win, WindowModeRequest::Tiling);
        arrange(ctx, Some(monitor_id), ArrangeAnimation::Configured);
    } else {
        // Window is already tiled and not dropped on a tag — nothing to do.
        return;
    }

    // ── Correct the saved placement using pre-drag dimensions ─────────────
    //
    // Keep the drop position (x/y from set_window_mode's saved client.geo), but
    // preserve the pre-drag floating size so un-tiling restores dimensions.
    if was_floating && let Some(client) = ctx.core_mut().state.model.client_mut(win) {
        client.update_saved_floating_size(grab_start_rect.size());
    }
}

/// Commit the same destination policy used by active motion and its preview.
pub fn complete_move_drop(
    ctx: &mut WmCtx,
    win: WindowId,
    grab_start_rect: Rect,
    root: Point,
    free_geometry: Rect,
    modifiers: ModMask,
) {
    // Resolve the dropped client once: the destination policy and the
    // tiling/border decision below both read it.
    let resolved = {
        let model = &ctx.core().state.model;
        model.client(win).map(|client| {
            (
                resolve_move_drop(model, client, root),
                client.mode().is_normal_tiling(),
                client.old_border_width,
            )
        })
    };
    let Some((target, is_normal_tiling, old_border_width)) = resolved else {
        ctx.update_layout_preview(None);
        return;
    };
    if matches!(target, Some(MoveDropTarget::Tree(_))) {
        let _ = crate::layouts::place_tree_at_point(ctx, win, root);
    } else if matches!(target, Some(MoveDropTarget::Bar(_))) {
        handle_bar_drop(ctx, win, grab_start_rect, Some(root), modifiers);
    } else if let Some(target) = target {
        let monitor_id = target.monitor();
        // Membership on the destination is a hash lookup, not a scan of every
        // output. A tiled source stayed in its slot during preview. Apply its
        // free destination geometry only on commit, keeping cancellation
        // lossless.
        let already_on_destination = ctx
            .core()
            .state
            .model
            .monitor(monitor_id)
            .is_some_and(|monitor| monitor.has_client(win));
        if !already_on_destination {
            let _ = transfer_client(ctx, win, monitor_id, TransferFocus::FollowWindow);
        }
        // The transfer cannot change the destination's layout or work area,
        // and neither can the border/geometry updates below: read both once.
        let (destination_layout, work) = {
            let monitor = ctx
                .core()
                .state
                .model
                .monitor(monitor_id)
                .expect("drop destination resolved its monitor");
            (monitor.current_layout(), monitor.work_rect())
        };
        if is_normal_tiling && destination_layout == PresentationMode::Floating {
            ctx.set_border(win, old_border_width);
        }
        ctx.move_resize(
            win,
            free_geometry,
            crate::geometry::MoveResizeOptions::hinted_immediate(false),
        );
        if let MoveDropTarget::Snap(_, edge) = target {
            // The destination owns the window by now, so the saved placement
            // reaches it through that monitor's own client map.
            if let Some(client) = ctx
                .core_mut()
                .state
                .model
                .monitor_mut(monitor_id)
                .and_then(|monitor| monitor.client_mut(win))
            {
                client.save_floating_placement(free_geometry, work);
                client.snap_status = edge;
                if let Some(rect) = edge.target_rect(client.border_width, work) {
                    ctx.move_resize(
                        win,
                        rect,
                        crate::geometry::MoveResizeOptions::hinted_immediate(false),
                    );
                }
            }
        }
        arrange(ctx, Some(monitor_id), ArrangeAnimation::Configured);
    }
    ctx.update_layout_preview(None);
}

/// Helper function for promoting a window to floating.
/// Used by both title drag and move operations.
pub fn promote_to_floating(
    ctx: &mut WmCtx,
    win: WindowId,
    intent: FloatingPlacementIntent,
) -> Option<(Rect, bool)> {
    // Literal maximization in the global floating presentation restores its
    // free geometry first. Re-evaluating placement afterwards preserves a
    // tiled client's manual-tree membership while still allowing it to move
    // freely in that presentation.
    crate::client::fullscreen::leave_maximized(ctx, win);
    promote_restored_client(ctx, win, intent)
}

/// The promotion half of [`promote_to_floating`], for a caller that already
/// ran `crate::client::fullscreen::leave_maximized` for this event.
pub(crate) fn promote_restored_client(
    ctx: &mut WmCtx,
    win: WindowId,
    intent: FloatingPlacementIntent,
) -> Option<(Rect, bool)> {
    // One view decides everything: floating clients keep their geometry, and
    // tiled clients inside a floating presentation move freely without
    // changing their persistent placement mode, so returning to tiling can
    // restore the manual tree.
    let monitor_id = {
        let view = ctx.core().state.model.client_view(win)?;
        if view.client.mode().is_normal_floating() {
            return Some((view.client.geo, false));
        }
        if view.monitor.current_layout() == PresentationMode::Floating
            && view.client.mode().is_normal_tiling()
        {
            return Some((view.client.geo, false));
        }
        view.monitor.id()
    };

    let restored_geometry = match set_window_mode(ctx, win, WindowModeRequest::Floating(intent)) {
        crate::floating::WindowModeChange::ChangedToFloating { restored_geometry } => {
            restored_geometry
        }
        crate::floating::WindowModeChange::MissingClient => return None,
        crate::floating::WindowModeChange::ChangedToTiling => {
            unreachable!("requesting floating mode produced a tiling transition")
        }
    };
    arrange(ctx, Some(monitor_id), ArrangeAnimation::Configured);
    Some((restored_geometry, true))
}

#[cfg(test)]
mod tests {
    use super::promote_to_floating;
    use crate::test_support::TestWm as Wm;

    use crate::client::geometry::FloatingPlacementIntent;
    use crate::layouts::PresentationMode;
    use crate::test_support::{MonitorBuilder, add_client, add_client_with};
    use crate::types::{Client, ClientMode, ClientPlacement, MonitorId, Rect, TagMask, WindowId};

    /// Push the 1200x800 monitor these drop fixtures sit on.
    fn push_drop_monitor(wm: &mut Wm, available: Rect) -> MonitorId {
        wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .rect(Rect::new(0, 0, 1200, 800), available)
                .build(),
        )
    }

    #[test]
    fn floating_presentation_drag_does_not_change_tiled_placement() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        let monitor_id = push_drop_monitor(&mut wm, Rect::new(0, 0, 1200, 800));
        wm.core.state.model.monitors.set_selected(monitor_id);
        wm.core
            .state
            .model
            .monitors
            .get_mut(monitor_id)
            .unwrap()
            .per_tag_state()
            .presentation = PresentationMode::Floating;
        let win = WindowId(42);
        add_client_with(&mut wm.core.state.model, monitor_id, |client| {
            client.win = win;
            client.tags = TagMask::single(1).unwrap();
            client.mode = ClientMode::tiled();
            client.geo = Rect::new(100, 100, 400, 300);
        });

        let result = promote_to_floating(
            &mut wm.test_ctx(),
            win,
            FloatingPlacementIntent::PreservePointerAnchor(crate::types::Point::new(200, 200)),
        );

        assert_eq!(result, Some((Rect::new(100, 100, 400, 300), false)));
        assert_eq!(
            wm.core.state.model.client(win).unwrap().mode(),
            ClientMode::tiled()
        );
    }

    #[test]
    fn dragging_client_maximized_floating_window_restores_its_float_geometry() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        let work_rect = Rect::new(0, 30, 1200, 770);
        let monitor_id = push_drop_monitor(&mut wm, work_rect);
        wm.core.state.model.monitors.set_selected(monitor_id);
        let win = WindowId(43);
        let saved = Rect::new(220, 170, 680, 480);
        let mut client = Client {
            win,
            tags: TagMask::single(1).unwrap(),
            mode: ClientMode::maximized(ClientPlacement::Floating),
            geo: work_rect,
            ..Client::default()
        };
        client.save_floating_placement(saved, work_rect);
        add_client(&mut wm.core.state.model, monitor_id, client);

        let result = promote_to_floating(
            &mut wm.test_ctx(),
            win,
            FloatingPlacementIntent::PreservePointerAnchor(crate::types::Point::new(600, 200)),
        );

        assert_eq!(result, Some((saved, false)));
        let client = wm.core.state.model.client(win).unwrap();
        assert!(client.mode().is_normal_floating());
        assert_eq!(
            wm.core.state.model.client_protocol_maximized(win),
            Some(false)
        );
        assert_eq!(client.geo, saved);
    }
}

#[cfg(test)]
mod destination_tests {
    use super::*;
    use crate::test_support::TestWm as Wm;
    use crate::test_support::{MonitorBuilder, add_selected_client_with};

    fn fixture() -> (Wm, WindowId, MonitorId, MonitorId) {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.config.animations.enabled = false;
        wm.core.state.model.tags.num_tags = 9;
        let a = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .rect(Rect::new(0, 0, 800, 600), Rect::new(0, 0, 800, 600))
                .tag_count(9)
                .selected_tags(TagMask::single(1).unwrap())
                .build(),
        );
        let b = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .rect(Rect::new(800, 0, 800, 600), Rect::new(800, 0, 800, 600))
                .bar(30, true)
                .tag_count(9)
                .selected_tags(TagMask::single(2).unwrap())
                .build(),
        );
        wm.core.state.model.monitors.set_selected(a);
        let win = WindowId(981);
        add_selected_client_with(&mut wm.core.state.model, a, |c| {
            c.win = win;
            c.tags = TagMask::single(1).unwrap();
            c.mode = ClientMode::tiled();
            c.geo = Rect::new(0, 0, 800, 600);
        });
        for (index, tag) in wm
            .core
            .state
            .model
            .monitor_mut(b)
            .unwrap()
            .tags
            .iter_mut()
            .enumerate()
        {
            tag.name = (index + 1).to_string();
        }
        crate::bar::render_hit_caches_for_test(wm.test_ctx().core_mut());
        (wm, win, a, b)
    }

    #[test]
    fn destination_bar_transfers_and_tags_captured_window_without_stealing_focus() {
        let (mut wm, win, a, b) = fixture();
        let other = WindowId(982);
        add_selected_client_with(&mut wm.core.state.model, a, |c| {
            c.win = other;
            c.tags = TagMask::single(1).unwrap();
        });
        // Find tag 3 through the bar's real hit testing, rather than duplicating its metrics.
        let root = (800..1600)
            .map(|x| Point::new(x, 10))
            .find(|point| {
                let ctx = wm.test_ctx();
                let mon = ctx.core().state.model.monitor(b).unwrap();
                crate::bar::model::bar_position_at_x(
                    mon,
                    ctx.core(),
                    mon.local_work_point(*point).x,
                ) == BarPosition::Tag(2)
            })
            .unwrap();
        assert_eq!(
            resolve_move_drop(
                &wm.core.state.model,
                wm.core.state.model.client(win).unwrap(),
                root
            ),
            Some(MoveDropTarget::Bar(b))
        );
        handle_bar_drop(
            &mut wm.test_ctx(),
            win,
            Rect::new(0, 0, 800, 600),
            Some(root),
            ModMask::NONE,
        );
        assert_eq!(wm.core.state.model.monitor_of_client(win), Some(b));
        assert_eq!(
            wm.core.state.model.client(win).unwrap().tags,
            TagMask::single(3).unwrap()
        );
        assert_eq!(
            wm.core.state.model.client(other).unwrap().tags,
            TagMask::single(1).unwrap()
        );
        assert_eq!(wm.core.state.model.selected_monitor_id(), a);
        assert_eq!(wm.core.state.model.selected_win(), Some(other));
        assert_eq!(
            wm.core.state.model.monitor(b).unwrap().selected_tags(),
            TagMask::single(2).unwrap()
        );
    }

    #[test]
    fn alt_destination_bar_drop_follows_the_window_to_its_tag() {
        let (mut wm, win, _, b) = fixture();
        let root = (800..1600)
            .map(|x| Point::new(x, 10))
            .find(|point| {
                let ctx = wm.test_ctx();
                let mon = ctx.core().state.model.monitor(b).unwrap();
                crate::bar::model::bar_position_at_x(
                    mon,
                    ctx.core(),
                    mon.local_work_point(*point).x,
                ) == BarPosition::Tag(2)
            })
            .unwrap();
        handle_bar_drop(
            &mut wm.test_ctx(),
            win,
            Rect::new(0, 0, 800, 600),
            Some(root),
            ModMask::from_modifier(Modifier::Alt),
        );
        assert_eq!(wm.core.state.model.monitor_of_client(win), Some(b));
        assert_eq!(wm.core.state.model.selected_monitor_id(), b);
        assert_eq!(wm.core.state.model.selected_win(), Some(win));
        assert_eq!(
            wm.core.state.model.monitor(b).unwrap().selected_tags(),
            TagMask::single(3).unwrap()
        );
    }

    #[test]
    fn floating_edge_drop_snaps_on_destination_and_preserves_restore_geometry() {
        let (mut wm, win, _, b) = fixture();
        wm.core
            .state
            .model
            .client_mut(win)
            .unwrap()
            .set_placement(ClientPlacement::Floating);
        wm.core
            .state
            .model
            .monitor_mut(b)
            .unwrap()
            .per_tag_state()
            .presentation = PresentationMode::Floating;
        let root = Point::new(1599, 300);
        let free = Rect::new(1300, 150, 300, 200);
        assert_eq!(
            resolve_move_drop(
                &wm.core.state.model,
                wm.core.state.model.client(win).unwrap(),
                root
            ),
            Some(MoveDropTarget::Snap(b, SnapPosition::Right))
        );
        complete_move_drop(&mut wm.test_ctx(), win, free, root, free, ModMask::NONE);
        let client = wm.core.state.model.client(win).unwrap();
        assert_eq!(wm.core.state.model.monitor_of_client(win), Some(b));
        assert_eq!(client.snap_status, SnapPosition::Right);
        assert_eq!(client.saved_floating_rect(), Some(free));
        let expected = SnapPosition::Right
            .target_rect(
                client.border_width,
                wm.core.state.model.monitor(b).unwrap().work_rect(),
            )
            .unwrap();
        assert_eq!(client.geo, expected);
    }
}
