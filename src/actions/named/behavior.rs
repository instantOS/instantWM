use std::collections::HashMap;

use crate::config::ModeConfig;
use crate::config::config_toml::{HorizontalEdge, VerticalEdge};
use crate::contexts::WmCtx;
use crate::floating::{
    DEFAULT_EDGE_SCRATCHPAD_NAME, key_move, key_resize, set_scratchpad_direction,
};
use crate::focus::{direction_focus, focus_stack_neighbor};
use crate::layouts::tree::Side;
use crate::layouts::{
    MaximizedStackReorder, focus_tree_neighbor, reorder_maximized_stack, resize_tree,
    swap_tree_neighbor,
};
use crate::tags::move_client_follow_view;
use crate::types::{EdgeDirection, HorizontalDirection, VerticalDirection};

impl From<HorizontalDirection> for Side {
    fn from(direction: HorizontalDirection) -> Self {
        match direction {
            HorizontalDirection::Left => Side::Left,
            HorizontalDirection::Right => Side::Right,
        }
    }
}

impl From<VerticalDirection> for Side {
    fn from(direction: VerticalDirection) -> Self {
        match direction {
            VerticalDirection::Up => Side::Top,
            VerticalDirection::Down => Side::Bottom,
        }
    }
}

pub(super) fn validate_mode_name(
    configured_modes: &HashMap<String, ModeConfig>,
    name: &str,
) -> Result<(), String> {
    if name == crate::core_state::TREE_PLACEMENT_MODE_NAME {
        return Err("mode 'placement' can only be entered by begin_tree_placement".to_string());
    }
    if configured_modes.contains_key(name)
        || matches!(
            crate::core_state::ActiveWmMode::from_name(name),
            crate::core_state::ActiveWmMode::Default | crate::core_state::ActiveWmMode::Overview
        )
    {
        Ok(())
    } else {
        Err(format!("mode '{name}' not found"))
    }
}

pub(super) fn focus_horizontal(ctx: &mut WmCtx<'_>, direction: HorizontalDirection) {
    // The overview is its own grid projection with a self-contained
    // directional model, so `focus.horizontal_edge` deliberately does not
    // apply inside it.
    if ctx.core().model().is_overview_active() {
        crate::overview::focus_direction(ctx, direction.into());
        return;
    }

    let edge = ctx.core().config().focus.horizontal_edge;
    let maximized = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_maximized_layout();

    if maximized {
        // Maximized windows all occupy the same region, so there is no
        // geometry to walk: the bar/tree order is the only order a user can
        // perceive, and it is also what the boundary is measured against.
        // `Wrap` cycles that order — which is both the ordinary step and the
        // boundary resolved — while `overflow` and `none` stop at its end.
        if focus_stack_neighbor(ctx, direction.into(), edge == HorizontalEdge::Wrap) {
            return;
        }
    } else if focus_tree_neighbor(ctx, direction.into()) || direction_focus(ctx, direction.into()) {
        return;
    }

    horizontal_edge_action(ctx, edge, direction, maximized);
}

/// Apply the configured [`HorizontalEdge`] policy where a horizontal focus
/// step ran out of windows.
///
/// Called only once directional focus has run out of windows on the current
/// tag, so the policy decides what a boundary key press does rather than
/// whether it moves focus at all. `maximized` selects the boundary model:
/// ordinary tiling has real geometry to wrap across, maximized presentation
/// has none.
fn horizontal_edge_action(
    ctx: &mut WmCtx<'_>,
    edge: HorizontalEdge,
    direction: HorizontalDirection,
    maximized: bool,
) {
    match edge {
        // Traditional window manager behaviour: carry on into the next tag.
        HorizontalEdge::Overflow => crate::animation::scroll_view_with_slide(ctx, direction),
        // Maximized presentation stacks its windows in the same region, so
        // there is no meaningful "leftmost" to land on; the bar/tree order is
        // the only cycle a user can perceive there, and `focus_horizontal`
        // already ran it. Getting here means that order was too short to move
        // through at all, which is genuinely nowhere to go.
        HorizontalEdge::Wrap if maximized => {}
        HorizontalEdge::Wrap => {
            let _ = crate::focus::wrap_direction_focus(ctx, direction.into());
        }
        HorizontalEdge::None => {}
    }
}

pub(super) fn focus_vertical(ctx: &mut WmCtx<'_>, direction: VerticalDirection) {
    // Same deliberate bypass as `focus_horizontal`: the overview owns its own
    // directional model, so neither edge policy applies inside it.
    if ctx.core().model().is_overview_active() {
        crate::overview::focus_direction(ctx, direction.into());
        return;
    }

    let edge = ctx.core().config().focus.vertical_edge;
    let maximized = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_maximized_layout();

    if maximized {
        // Maximized windows overlap, so the tree and geometry have nothing to
        // offer: the bar order is the only order there is to move through, and
        // it is also what the boundary is measured against. `Wrap` cycles that
        // order; `none` stops at its end.
        if focus_stack_neighbor(ctx, direction.into(), edge == VerticalEdge::Wrap) {
            return;
        }
    } else if focus_tree_neighbor(ctx, direction.into()) || direction_focus(ctx, direction.into()) {
        return;
    }

    vertical_edge_action(ctx, edge, direction, maximized);
}

/// Apply the configured [`VerticalEdge`] policy where a vertical focus step
/// ran out of windows.
///
/// The vertical counterpart of `horizontal_edge_action`, minus `overflow`:
/// tags do not stack, so there is no adjacent workspace to carry on into.
/// `wrap` resolves against real geometry in ordinary tiling — running off the
/// top edge lands on the bottom-most window — while maximized presentation
/// has no geometry to wrap across and cycles the bar order instead.
fn vertical_edge_action(
    ctx: &mut WmCtx<'_>,
    edge: VerticalEdge,
    direction: VerticalDirection,
    maximized: bool,
) {
    match edge {
        // The bar-order cycle already ran in `focus_vertical`; getting here
        // means that order was too short to move through at all.
        VerticalEdge::Wrap if maximized => {}
        VerticalEdge::Wrap => {
            let _ = crate::focus::wrap_direction_focus(ctx, direction.into());
        }
        VerticalEdge::None => {}
    }
}

pub(super) fn move_horizontal(ctx: &mut WmCtx<'_>, direction: HorizontalDirection) {
    match reorder_maximized_stack(ctx, direction.into()) {
        MaximizedStackReorder::Reordered | MaximizedStackReorder::ReconcileRequired => return,
        MaximizedStackReorder::Boundary => {
            let _ = move_client_follow_view(ctx, direction);
            return;
        }
        MaximizedStackReorder::NotApplicable => {}
    }

    if swap_tree_neighbor(ctx, direction.into()) {
        return;
    }
    let Some(win) = ctx.core().model().selected_win() else {
        return;
    };
    if !key_move(ctx, win, direction.into()) {
        let _ = move_client_follow_view(ctx, direction);
    }
}

pub(super) fn move_vertical(ctx: &mut WmCtx<'_>, direction: VerticalDirection) {
    if !matches!(
        reorder_maximized_stack(ctx, direction.into()),
        MaximizedStackReorder::NotApplicable
    ) {
        return;
    }

    if !swap_tree_neighbor(ctx, direction.into())
        && let Some(win) = ctx.core().model().selected_win()
    {
        key_move(ctx, win, direction.into());
    }
}

pub(super) fn key_resize_or_tree(
    ctx: &mut WmCtx<'_>,
    side: Side,
    direction: crate::types::Direction,
) {
    if !resize_tree(ctx, side)
        && let Some(win) = ctx.core().model().selected_win()
    {
        key_resize(ctx, win, direction);
    }
}

/// Logical pixels added per unconfigured gap key press. Bindings can pass an
/// explicit integer argument to use a different step.
pub(super) const DEFAULT_GAP_STEP: i32 = 2;

/// Move both tiling gaps by `delta` logical pixels and re-arrange.
///
/// Inner and outer gaps move together because users think of "the gap size"
/// as one knob; per-axis values stay reachable through config and IPC. Both
/// clamp at zero: placement treats zero gaps as disabled, so decreasing at
/// the floor simply keeps gapless tiling instead of inverting windows.
pub(super) fn adjust_gaps(ctx: &mut WmCtx<'_>, delta: i32) {
    let layout = &mut ctx.core_mut().config_mut().layout;
    layout.inner_gap = layout.inner_gap.saturating_add(delta).max(0);
    layout.outer_gap = layout.outer_gap.saturating_add(delta).max(0);
    crate::layouts::manager::arrange(ctx, None);
}

pub(super) fn with_selected_win(
    ctx: &mut WmCtx<'_>,
    f: impl FnOnce(&mut WmCtx<'_>, crate::types::WindowId),
) {
    if let Some(win) = ctx.core().model().selected_win() {
        f(ctx, win);
    }
}

pub(super) fn edge_scratchpad_set_direction(ctx: &mut WmCtx, dir: EdgeDirection) {
    if let Some(win) = ctx
        .core()
        .model()
        .scratchpad_find(DEFAULT_EDGE_SCRATCHPAD_NAME)
    {
        set_scratchpad_direction(ctx, win, dir);
    }
}
