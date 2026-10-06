use crate::contexts::WmCtx;
use crate::layouts::ArrangeAnimation;
use crate::layouts::PresentationMode;
use crate::layouts::tree::TreePlacementSession;
use crate::types::{MonitorId, Rect, TagMask, WindowId};

use super::arrange::{TilingContext, arrange};
use super::commands::finish_layout_change_for_monitor;

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
    model: &crate::model::WmModel,
    window: WindowId,
    point: crate::types::Point,
) -> Option<PointerTreeResizeStart> {
    if !uses_manual_tree_pointer_interaction(model, window) {
        return None;
    }
    let view = model.client_view(window)?;
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
    state: &crate::core_state::CoreState,
    point: crate::types::Point,
) -> Option<(WindowId, PointerTreeResizeStart)> {
    let model = &state.model;
    let monitor = model
        .monitors
        .monitor_intersecting_rect(crate::mouse::pointer::point_rect(point))?;
    if monitor.current_layout() != PresentationMode::Tiled {
        return None;
    }
    let visible_tags = monitor.visible_tags();
    if monitor
        .iter_clients()
        .any(|(_, client)| client.is_visible(visible_tags) && client.geo.contains_point(point))
    {
        return None;
    }

    let tiling = TilingContext::for_monitor(
        monitor,
        &state.config.layout,
        state.config.window.resize_hints,
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

    pointer_tree_resize_start(&state.model, win, point).map(|resize| (win, resize))
}

/// Whether pointer movement/resizing should edit the persistent layout tree.
///
/// Every tiled move remains a placement gesture, including a lone source.
/// Maximized and floating presentations retain free movement.
pub(crate) fn uses_manual_tree_pointer_interaction(
    model: &crate::model::WmModel,
    window: WindowId,
) -> bool {
    model.client_view(window).is_some_and(|view| {
        view.monitor.current_layout() == PresentationMode::Tiled
            && view.client.mode().is_normal_tiling()
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
        let view = match core.state.model.client_view(window) {
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
            &core.state.config.layout,
            core.state.config.window.resize_hints,
        );
        (tiling.work_rect(), view.monitor.id())
    };
    let minimum_weight = ctx.core().state.config.layout.minimum_weight;
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
        .state
        .model
        .monitor_mut(monitor_id)
        .expect("client view guaranteed its monitor exists")
        .per_tag_state()
        .layout_tree = candidate;
    arrange(ctx, Some(monitor_id), ArrangeAnimation::Immediate);
    true
}

/// Tiling geometry of the selected monitor.
pub(crate) fn selected_tiling(state: &crate::core_state::CoreState) -> TilingContext {
    TilingContext::for_monitor(
        state.model.expect_selected_monitor(),
        &state.config.layout,
        state.config.window.resize_hints,
    )
}

/// The pointer placement session for dragging `window` over the pointer's
/// monitor, reusing the cached one while it still describes the same view.
fn pointer_placement(
    state: &mut crate::core_state::CoreState,
    window: WindowId,
    point: crate::types::Point,
) -> Option<&mut PointerPlacementPreviewCache> {
    let monitor = state
        .model
        .monitors
        .monitor_intersecting_rect(Rect::new(point.x, point.y, 1, 1))?;
    if monitor.current_layout() != PresentationMode::Tiled {
        return None;
    }
    let (monitor_id, tags) = (monitor.id(), monitor.selected_tags());
    let cached = state
        .interaction
        .pointer_placement_cache
        .as_ref()
        .is_some_and(|cache| {
            cache.session.source() == window && cache.monitor_id == monitor_id && cache.tags == tags
        });
    if !cached {
        let tree = monitor.per_tag()?.layout_tree.clone();
        let client = state.model.client(window)?;
        let incoming =
            (state.model.monitor_of_client(window) != Some(monitor_id)).then_some(client);
        let tiling = TilingContext::for_drop(
            monitor,
            &state.config.layout,
            state.config.window.resize_hints,
            incoming,
        );
        let session = TreePlacementSession::new(
            tree,
            window,
            tiling.work_rect(),
            state.config.layout.pointer_edge_fraction,
            tiling.minimums.clone(),
        );
        state.interaction.pointer_placement_cache = Some(PointerPlacementPreviewCache {
            monitor_id,
            tags,
            tiling,
            session,
        });
    }
    state.interaction.pointer_placement_cache.as_mut()
}

pub fn place_tree_at_point(
    ctx: &mut WmCtx<'_>,
    window: WindowId,
    point: crate::types::Point,
) -> bool {
    let Some(source) = ctx.core().state.model.monitor_of_client(window) else {
        return false;
    };
    if pointer_placement(&mut ctx.core_mut().state, window, point).is_none() {
        return false;
    }
    let cache = ctx
        .core_mut()
        .state
        .interaction
        .pointer_placement_cache
        .take()
        .unwrap();
    let Some(plan) = cache.session.into_plan(point) else {
        return false;
    };
    let target = cache.monitor_id;
    if source != target
        && crate::monitor::transfer_client(
            ctx,
            window,
            target,
            crate::monitor::TransferFocus::FollowWindow,
        )
        .is_none()
    {
        return false;
    }
    ctx.core_mut()
        .state
        .model
        .monitor_mut(target)
        .unwrap()
        .per_tag_state()
        .layout_tree = plan.into_tree();
    if source != target {
        finish_layout_change_for_monitor(ctx, source);
    }
    finish_layout_change_for_monitor(ctx, target);
    true
}

/// Compute the exact final outer rectangle for a tiled pointer drop without
/// changing the tree. Returns `None` when the point is not a valid target.
pub fn preview_tree_at_point(
    state: &mut crate::core_state::CoreState,
    window: WindowId,
    point: crate::types::Point,
) -> Option<Rect> {
    if !state
        .model
        .client(window)
        .is_some_and(|client| client.mode().is_normal_tiling())
    {
        return None;
    }
    pointer_placement(state, window, point)?;
    let cache = state.interaction.pointer_placement_cache.as_mut()?;
    let slot = cache.session.preview_point(point)?;
    let client = state.model.client(window)?;
    Some(
        cache
            .tiling
            .outer_rect(client, slot, state.config.window.resize_hints),
    )
}

#[cfg(test)]
mod outline_cache_tests {
    use super::*;
    use crate::core_state::ActiveWmMode;
    use crate::layouts::tree::LayoutTree;
    use crate::types::InteractionOutlineStyle;

    #[test]
    fn clearing_an_already_hidden_outline_discards_stale_placement() {
        let mut core = crate::core_state::CoreState::default();
        let monitor_id = core.model.monitors.push(
            crate::test_support::MonitorBuilder::new()
                .rect(Rect::new(0, 0, 800, 600), Rect::new(0, 0, 800, 600))
                .tag_count(1)
                .selected_tags(TagMask::single(1).unwrap())
                .build(),
        );
        let tiling = crate::layouts::manager::selected_tiling(&core);
        let session = TreePlacementSession::new(
            LayoutTree::default(),
            WindowId(1),
            tiling.work_rect(),
            0.34,
            tiling.minimums.clone(),
        );
        core.interaction.pointer_placement_cache =
            Some(crate::layouts::manager::PointerPlacementPreviewCache {
                monitor_id,
                tags: TagMask::single(1).unwrap(),
                tiling,
                session,
            });
        assert!(
            core.interaction
                .update_outline(
                    &core.config.animations,
                    &ActiveWmMode::Default,
                    None,
                    InteractionOutlineStyle::Layout
                )
                .is_none()
        );
        assert!(core.interaction.pointer_placement_cache.is_none());
    }
}
