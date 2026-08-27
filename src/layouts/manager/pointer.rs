use crate::contexts::WmCtx;
use crate::layouts::PresentationMode;
use crate::layouts::placement::LayoutPlacement;
use crate::types::{MonitorId, Rect, Size, TagMask, WindowId};
use std::collections::HashMap;

use super::arrange::{arrange, compute_tiling_constraints, compute_tiling_geometry};
use super::finish_layout_change;

#[derive(Debug, Clone)]
pub(crate) struct PointerPlacementPreviewCache {
    source: WindowId,
    monitor_id: MonitorId,
    tags: TagMask,
    edge_fraction: f64,
    placement: LayoutPlacement,
    session: crate::layouts::tree::TreePlacementSession,
}

impl PointerPlacementPreviewCache {
    fn matches(
        &self,
        source: WindowId,
        monitor_id: MonitorId,
        tags: TagMask,
        edge_fraction: f64,
    ) -> bool {
        self.source == source
            && self.monitor_id == monitor_id
            && self.tags == tags
            && self.edge_fraction.to_bits() == edge_fraction.to_bits()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PointerTreeResizeStart {
    pub direction: crate::types::ResizeDirection,
    pub origin: crate::layouts::tree::LayoutTree,
}

/// Prepare a Super+right-button tree resize, or return `None` when the
/// ordinary floating-resize behavior should be used instead.
pub(crate) fn pointer_tree_resize_start(
    ctx: &WmCtx<'_>,
    window: WindowId,
    point: crate::types::Point,
) -> Option<PointerTreeResizeStart> {
    let view = ctx.core().model().client_view(window)?;
    let tiled_count = view
        .monitor
        .collect_tiled(&ctx.core().model().clients)
        .len();
    if !manual_tree_pointer_interaction_allowed(
        view.monitor.current_layout(),
        view.client.mode().is_normal_tiling(),
        tiled_count,
    ) {
        return None;
    }
    let tree = &view.monitor.per_tag()?.layout_tree;
    let left = tree.can_resize_side(window, crate::layouts::tree::Side::Left);
    let right = tree.can_resize_side(window, crate::layouts::tree::Side::Right);
    let top = tree.can_resize_side(window, crate::layouts::tree::Side::Top);
    let bottom = tree.can_resize_side(window, crate::layouts::tree::Side::Bottom);
    let horizontal = left || right;
    let vertical = top || bottom;
    if !pointer_tree_resize_allowed(
        view.monitor.current_layout(),
        view.client.mode().is_normal_tiling(),
        tiled_count,
        horizontal,
        vertical,
    ) {
        return None;
    }
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
        origin: tree.clone(),
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
    let tiled = monitor.collect_tiling_tree_members(&model.clients);
    if monitor.current_layout() != PresentationMode::Tiled || tiled.len() <= 1 {
        return None;
    }
    let visible_tags = monitor.visible_tags();
    if monitor
        .iter_clients(&model.clients)
        .any(|(_, client)| client.is_visible(visible_tags) && client.geo.contains_point(point))
    {
        return None;
    }

    let geom = compute_tiling_geometry(
        monitor,
        &model.clients,
        &ctx.core().config().layout,
        ctx.core().config().window.resize_hints,
        ctx.core().derived().bar_height,
    )?;
    if geom.placement.inner_gap() <= 0 || !geom.placement.work_rect().contains_point(point) {
        return None;
    }

    let (win, slot) = geom
        .slots
        .into_iter()
        .find(|(_, slot)| slot.contains_point(point))?;

    // Removing only the inner gap (and no client border) distinguishes a real
    // gap from content, borders, and unused pixels at the work-area edge.
    if geom.placement.client_rect(slot, 0).contains_point(point) {
        return None;
    }

    pointer_tree_resize_start(ctx, win, point).map(|resize| (win, resize))
}

pub(super) fn pointer_tree_resize_allowed(
    presentation: PresentationMode,
    client_is_tiled: bool,
    tiled_count: usize,
    horizontal: bool,
    vertical: bool,
) -> bool {
    manual_tree_pointer_interaction_allowed(presentation, client_is_tiled, tiled_count)
        && (horizontal || vertical)
}

pub(super) fn manual_tree_pointer_interaction_allowed(
    presentation: PresentationMode,
    client_is_tiled: bool,
    tiled_count: usize,
) -> bool {
    presentation == PresentationMode::Tiled && client_is_tiled && tiled_count > 1
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
    let Some(view) = model.client_view(window) else {
        return false;
    };
    manual_tree_pointer_interaction_allowed(
        view.monitor.current_layout(),
        view.client.mode().is_normal_tiling(),
        view.monitor.tiled_client_count(&model.clients),
    )
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

    let (layout_rect, minimum_weight, monitor_id) = {
        let view = match ctx.core().model().client_view(window) {
            Some(view)
                if view.monitor.current_layout() == PresentationMode::Tiled
                    && view.client.mode().is_normal_tiling()
                    && view.client.is_visible(view.monitor.visible_tags()) =>
            {
                view
            }
            _ => return false,
        };
        let tiled_count = view
            .monitor
            .collect_tiled(&ctx.core().model().clients)
            .len() as u32;
        let placement = LayoutPlacement::new(
            &ctx.core().config().layout,
            view.monitor,
            PresentationMode::Tiled,
            tiled_count,
        );
        (
            placement.work_rect(),
            ctx.core().config().layout.minimum_weight,
            view.monitor.id(),
        )
    };
    let mut candidate = origin.clone();
    let (left, right, top, bottom) = direction.affected_edges();
    if left || right {
        let side = if left { Side::Left } else { Side::Right };
        let _ = candidate.resize_edge_by_pixels(
            window,
            side,
            current.x - start.x,
            layout_rect,
            minimum_weight,
        );
    }
    if top || bottom {
        let side = if top { Side::Top } else { Side::Bottom };
        let _ = candidate.resize_edge_by_pixels(
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

pub(super) fn selected_tiling_constraints(
    ctx: &WmCtx<'_>,
) -> Option<(LayoutPlacement, HashMap<WindowId, Size>)> {
    let monitor = ctx.core().model().expect_selected_monitor();
    Some(compute_tiling_constraints(
        monitor,
        &ctx.core().model().clients,
        &ctx.core().config().layout,
        ctx.core().config().window.resize_hints,
        ctx.core().derived().bar_height,
    ))
}

fn selected_tree_placement_session(
    ctx: &WmCtx<'_>,
    source: WindowId,
) -> Option<(LayoutPlacement, crate::layouts::tree::TreePlacementSession)> {
    let (placement, minimums) = selected_tiling_constraints(ctx)?;
    let tree = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .per_tag()?
        .layout_tree
        .clone();
    let session = crate::layouts::tree::TreePlacementSession::new(
        tree,
        source,
        placement.work_rect(),
        ctx.core().config().layout.pointer_edge_fraction,
        minimums,
    );
    Some((placement, session))
}

pub(crate) fn tree_placement_targets(
    ctx: &WmCtx<'_>,
    source: WindowId,
) -> Vec<crate::layouts::tree::PlacementTarget> {
    selected_tree_placement_session(ctx, source)
        .map(|(_, session)| session.targets())
        .unwrap_or_default()
}

pub(crate) fn preview_tree_target(
    ctx: &WmCtx<'_>,
    source: WindowId,
    target: crate::layouts::tree::PlacementTarget,
) -> Option<(LayoutPlacement, Rect)> {
    let (placement, session) = selected_tree_placement_session(ctx, source)?;
    let plan = session.plan_target(target)?;
    Some((placement, plan.source_slot()))
}

pub(crate) fn apply_tree_target(
    ctx: &mut WmCtx<'_>,
    source: WindowId,
    target: crate::layouts::tree::PlacementTarget,
) -> bool {
    let Some((_, session)) = selected_tree_placement_session(ctx, source) else {
        return false;
    };
    let Some(plan) = session.plan_target(target) else {
        return false;
    };
    ctx.core_mut()
        .model_mut()
        .expect_selected_monitor_mut()
        .per_tag_state()
        .layout_tree = plan.into_tree();
    true
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
    {
        return false;
    }
    let monitor = ctx.core().model().expect_selected_monitor();
    let monitor_id = monitor.id();
    let tags = monitor.selected_tags();
    let edge_fraction = ctx.core().config().layout.pointer_edge_fraction;
    let cache_matches = ctx
        .core()
        .state()
        .interaction
        .pointer_placement_cache
        .as_ref()
        .is_some_and(|cache| cache.matches(window, monitor_id, tags, edge_fraction));
    let cached_plan = if cache_matches {
        ctx.core_mut()
            .state_mut()
            .interaction
            .pointer_placement_cache
            .as_mut()
            .and_then(|cache| cache.session.plan_point(point))
    } else {
        None
    };
    let plan = match cached_plan {
        Some(plan) => plan,
        None => {
            let Some((_, mut session)) = selected_tree_placement_session(ctx, window) else {
                return false;
            };
            let Some(plan) = session.plan_point(point) else {
                return false;
            };
            plan
        }
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
    let monitor = ctx.core().model().expect_selected_monitor();
    if !monitor.is_tiling_layout()
        || !ctx
            .core()
            .model()
            .client(window)
            .is_some_and(|client| client.mode().is_normal_tiling())
    {
        return None;
    }
    let monitor_id = monitor.id();
    let tags = monitor.selected_tags();
    let edge_fraction = ctx.core().config().layout.pointer_edge_fraction;
    let cache_matches = ctx
        .core()
        .state()
        .interaction
        .pointer_placement_cache
        .as_ref()
        .is_some_and(|cache| cache.matches(window, monitor_id, tags, edge_fraction));
    if cache_matches {
        let (placement, slot) = {
            let cache = ctx
                .core_mut()
                .state_mut()
                .interaction
                .pointer_placement_cache
                .as_mut()?;
            (cache.placement, cache.session.preview_point(point)?)
        };
        return crate::layouts::keyboard_placement::tree_slot_outer_rect(
            ctx, window, placement, slot,
        );
    }

    let (placement, mut session) = selected_tree_placement_session(ctx, window)?;
    let slot = session.preview_point(point);
    ctx.core_mut()
        .state_mut()
        .interaction
        .pointer_placement_cache = Some(PointerPlacementPreviewCache {
        source: window,
        monitor_id,
        tags,
        edge_fraction,
        placement,
        session,
    });
    let slot = slot?;
    crate::layouts::keyboard_placement::tree_slot_outer_rect(ctx, window, placement, slot)
}
