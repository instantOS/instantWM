use crate::contexts::WmCtx;
use crate::layouts::PresentationMode;
use crate::layouts::tree::TreePlacementSession;
use crate::types::{MonitorId, Rect, TagMask, WindowId};

use super::arrange::{TilingContext, arrange};
use super::finish_layout_change;

/// Pointer placement state for one drag over one monitor/tag view. Every
/// authoritative arrange drops it, so its tiling snapshot cannot go stale.
#[derive(Debug, Clone)]
pub(crate) struct PointerPlacementPreviewCache {
    monitor_id: MonitorId,
    tags: TagMask,
    tiling: TilingContext,
    session: TreePlacementSession,
}

#[derive(Debug, Clone)]
pub(crate) struct PointerTreeResizeStart {
    pub direction: crate::types::ResizeDirection,
    pub origin: std::sync::Arc<crate::layouts::tree::LayoutTree>,
}

/// Prepare a Super+right-button tree resize, or return `None` when the
/// ordinary floating-resize behavior should be used instead.
pub(crate) fn pointer_tree_resize_start(
    ctx: &WmCtx<'_>,
    window: WindowId,
    point: crate::types::Point,
) -> Option<PointerTreeResizeStart> {
    if !uses_manual_tree_pointer_interaction(ctx.core().model(), window) {
        return None;
    }
    let view = ctx.core().model().client_view(window)?;
    let tree = &view.monitor.per_tag()?.layout_tree;
    let left = tree.can_resize_side(window, crate::layouts::tree::Side::Left);
    let right = tree.can_resize_side(window, crate::layouts::tree::Side::Right);
    let top = tree.can_resize_side(window, crate::layouts::tree::Side::Top);
    let bottom = tree.can_resize_side(window, crate::layouts::tree::Side::Bottom);
    let hit = view.client.geo.local_point(point);
    let requested = crate::types::ResizeDirection::from_hit(view.client.geo.size(), hit);
    let direction = available_tree_resize_direction(
        requested,
        left,
        right,
        top,
        bottom,
        hit,
        view.client.geo.size(),
    )?;
    Some(PointerTreeResizeStart {
        direction,
        origin: std::sync::Arc::new(tree.clone()),
    })
}

/// Resolve an adjustable tiled-tree seam from a pointer position in an inner
/// gap. Outer gaps deliberately do not count: they retain desktop semantics.
pub(crate) fn pointer_tree_gap_resize_start(
    ctx: &WmCtx<'_>,
    point: crate::types::Point,
) -> Option<(WindowId, PointerTreeResizeStart)> {
    let model = ctx.core().model();
    let monitor_id = model
        .monitors
        .id_intersecting_rect(crate::mouse::pointer::point_rect(point))?;
    let monitor = model.monitor(monitor_id)?;
    if monitor.current_layout() != PresentationMode::Tiled {
        return None;
    }
    let visible_tags = monitor.visible_tags();
    if monitor
        .iter_clients(&model.clients)
        .any(|(_, client)| client.is_visible(visible_tags) && client.geo.contains_point(point))
    {
        return None;
    }

    let tiling = TilingContext::for_monitor(
        monitor,
        &model.clients,
        &ctx.core().config().layout,
        ctx.core().config().window.resize_hints,
    );
    if tiling.members.len() <= 1
        || tiling.placement.inner_gap() <= 0
        || !tiling.work_rect().contains_point(point)
    {
        return None;
    }

    let (slots, _) = tiling.slots(&monitor.per_tag()?.layout_tree);
    let (win, slot) = slots
        .into_iter()
        .find(|(_, slot)| slot.contains_point(point))?;

    // Removing only the inner gap (and no client border) distinguishes a real
    // gap from content, borders, and unused pixels at the work-area edge.
    if tiling.placement.client_rect(slot, 0).contains_point(point) {
        return None;
    }

    pointer_tree_resize_start(ctx, win, point).map(|resize| (win, resize))
}

/// Whether pointer movement/resizing should edit the persistent layout tree.
///
/// A lone tiled client has no meaningful tree relationship to manipulate, and
/// maximized presentation deliberately hides those relationships. Both cases
/// therefore use the ordinary floating drag behavior.
pub(crate) fn uses_manual_tree_pointer_interaction(
    model: &crate::model::WmModel,
    window: WindowId,
) -> bool {
    model.client_view(window).is_some_and(|view| {
        view.monitor.current_layout() == PresentationMode::Tiled
            && view.client.mode().is_normal_tiling()
            && view.monitor.tiled_client_count(&model.clients) > 1
    })
}

pub(super) fn available_tree_resize_direction(
    requested: crate::types::ResizeDirection,
    can_left: bool,
    can_right: bool,
    can_top: bool,
    can_bottom: bool,
    hit: crate::types::Point,
    size: crate::types::Size,
) -> Option<crate::types::ResizeDirection> {
    use crate::types::ResizeDirection;

    let (left, right, top, bottom) = requested.affected_edges();
    let mut horizontal_edge = if left && can_left {
        Some(ResizeDirection::Left)
    } else if right && can_right {
        Some(ResizeDirection::Right)
    } else {
        None
    };
    let mut vertical_edge = if top && can_top {
        Some(ResizeDirection::Top)
    } else if bottom && can_bottom {
        Some(ResizeDirection::Bottom)
    } else {
        None
    };

    // A monitor-edge quadrant may not expose the requested seam. If neither
    // requested edge is adjustable, use the nearest actual seam; the returned
    // direction then accurately describes which edge will move.
    if horizontal_edge.is_none() && vertical_edge.is_none() {
        horizontal_edge = match (can_left, can_right) {
            (true, true) => Some(if hit.x < size.w / 2 {
                ResizeDirection::Left
            } else {
                ResizeDirection::Right
            }),
            (true, false) => Some(ResizeDirection::Left),
            (false, true) => Some(ResizeDirection::Right),
            (false, false) => None,
        };
        vertical_edge = match (can_top, can_bottom) {
            (true, true) => Some(if hit.y < size.h / 2 {
                ResizeDirection::Top
            } else {
                ResizeDirection::Bottom
            }),
            (true, false) => Some(ResizeDirection::Top),
            (false, true) => Some(ResizeDirection::Bottom),
            (false, false) => None,
        };
        if horizontal_edge.is_some() && vertical_edge.is_some() {
            let horizontal_distance = hit.x.min((size.w - hit.x).abs());
            let vertical_distance = hit.y.min((size.h - hit.y).abs());
            if horizontal_distance <= vertical_distance {
                vertical_edge = None;
            } else {
                horizontal_edge = None;
            }
        }
    }

    match (horizontal_edge, vertical_edge) {
        (Some(ResizeDirection::Left), Some(ResizeDirection::Top)) => Some(ResizeDirection::TopLeft),
        (Some(ResizeDirection::Right), Some(ResizeDirection::Top)) => {
            Some(ResizeDirection::TopRight)
        }
        (Some(ResizeDirection::Left), Some(ResizeDirection::Bottom)) => {
            Some(ResizeDirection::BottomLeft)
        }
        (Some(ResizeDirection::Right), Some(ResizeDirection::Bottom)) => {
            Some(ResizeDirection::BottomRight)
        }
        (Some(edge), None) | (None, Some(edge)) => Some(edge),
        _ => None,
    }
}

/// Re-evaluate a tiled resize from its immutable drag origin.
pub(crate) fn update_pointer_tree_resize(
    ctx: &mut WmCtx<'_>,
    window: WindowId,
    origin: &crate::layouts::tree::LayoutTree,
    direction: crate::types::ResizeDirection,
    start: crate::types::Point,
    current: crate::types::Point,
) -> bool {
    use crate::layouts::tree::Side;

    let (layout_rect, monitor_id) = {
        let core = ctx.core();
        let view = match core.model().client_view(window) {
            Some(view)
                if view.monitor.current_layout() == PresentationMode::Tiled
                    && view.client.mode().is_normal_tiling()
                    && view.client.is_visible(view.monitor.visible_tags()) =>
            {
                view
            }
            _ => return false,
        };
        let tiling = TilingContext::for_monitor(
            view.monitor,
            &core.model().clients,
            &core.config().layout,
            core.config().window.resize_hints,
        );
        (tiling.work_rect(), view.monitor.id())
    };
    let minimum_weight = ctx.core().config().layout.minimum_weight;
    let mut candidate = origin.clone();
    let (left, right, top, bottom) = direction.affected_edges();
    if left || right {
        let side = if left { Side::Left } else { Side::Right };
        candidate.resize_edge_by_pixels(
            window,
            side,
            current.x - start.x,
            layout_rect,
            minimum_weight,
        );
    }
    if top || bottom {
        let side = if top { Side::Top } else { Side::Bottom };
        candidate.resize_edge_by_pixels(
            window,
            side,
            current.y - start.y,
            layout_rect,
            minimum_weight,
        );
    }
    ctx.core_mut()
        .model_mut()
        .monitor_mut(monitor_id)
        .expect("client view guaranteed its monitor exists")
        .per_tag_state()
        .layout_tree = candidate;
    let animated = ctx.core().behavior().animated;
    if animated {
        ctx.core_mut().behavior_mut().animated = false;
    }
    arrange(ctx, Some(monitor_id));
    if animated {
        ctx.core_mut().behavior_mut().animated = true;
    }
    true
}

/// Tiling geometry of the selected monitor.
pub(crate) fn selected_tiling(ctx: &WmCtx<'_>) -> TilingContext {
    let core = ctx.core();
    TilingContext::for_monitor(
        core.model().expect_selected_monitor(),
        &core.model().clients,
        &core.config().layout,
        core.config().window.resize_hints,
    )
}

/// The pointer placement session for dragging `window` over the selected
/// monitor, reusing the cached one while it still describes the same view.
fn pointer_placement<'a>(
    ctx: &'a mut WmCtx<'_>,
    window: WindowId,
) -> Option<&'a mut PointerPlacementPreviewCache> {
    let monitor = ctx.core().model().expect_selected_monitor();
    let (monitor_id, tags) = (monitor.id(), monitor.selected_tags());
    let cached = ctx
        .core()
        .state()
        .interaction
        .pointer_placement_cache
        .as_ref()
        .is_some_and(|cache| {
            cache.session.source() == window && cache.monitor_id == monitor_id && cache.tags == tags
        });
    if !cached {
        let tree = monitor.per_tag()?.layout_tree.clone();
        let tiling = selected_tiling(ctx);
        let session = TreePlacementSession::new(
            tree,
            window,
            tiling.work_rect(),
            ctx.core().config().layout.pointer_edge_fraction,
            tiling.minimums.clone(),
        );
        ctx.core_mut()
            .state_mut()
            .interaction
            .pointer_placement_cache = Some(PointerPlacementPreviewCache {
            monitor_id,
            tags,
            tiling,
            session,
        });
    }
    ctx.core_mut()
        .state_mut()
        .interaction
        .pointer_placement_cache
        .as_mut()
}

pub fn place_tree_at_point(
    ctx: &mut WmCtx<'_>,
    window: WindowId,
    point: crate::types::Point,
) -> bool {
    if !ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_tiling_layout()
        || pointer_placement(ctx, window).is_none()
    {
        return false;
    }
    let Some(plan) = ctx
        .core_mut()
        .state_mut()
        .interaction
        .pointer_placement_cache
        .take()
        .and_then(|cache| cache.session.into_plan(point))
    else {
        return false;
    };
    ctx.core_mut()
        .model_mut()
        .expect_selected_monitor_mut()
        .per_tag_state()
        .layout_tree = plan.into_tree();
    finish_layout_change(ctx);
    true
}

/// Compute the exact final outer rectangle for a tiled pointer drop without
/// changing the tree. Returns `None` when the point is not a valid target.
pub fn preview_tree_at_point(
    ctx: &mut WmCtx<'_>,
    window: WindowId,
    point: crate::types::Point,
) -> Option<Rect> {
    if !ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_tiling_layout()
        || !ctx
            .core()
            .model()
            .client(window)
            .is_some_and(|client| client.mode().is_normal_tiling())
    {
        return None;
    }
    pointer_placement(ctx, window)?;
    let state = ctx.core_mut().state_mut();
    let cache = state.interaction.pointer_placement_cache.as_mut()?;
    let slot = cache.session.preview_point(point)?;
    let client = state.model.client(window)?;
    Some(
        cache
            .tiling
            .outer_rect(client, slot, state.config.window.resize_hints),
    )
}
