use std::collections::HashMap;

use crate::config::ModeConfig;
use crate::contexts::WmCtx;
use crate::floating::{
    DEFAULT_EDGE_SCRATCHPAD_NAME, key_move, key_resize, set_scratchpad_direction,
};
use crate::focus::{direction_focus, focus_stack, focus_stack_neighbor};
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
    if ctx.core().model().is_overview_active() {
        crate::overview::focus_direction(ctx, direction.into());
        return;
    }
    if ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_maximized_layout()
    {
        if !focus_stack_neighbor(ctx, direction.into()) {
            crate::animation::scroll_view_with_slide(ctx, direction);
        }
        return;
    }

    if !focus_tree_neighbor(ctx, direction.into()) && !direction_focus(ctx, direction.into()) {
        crate::animation::scroll_view_with_slide(ctx, direction);
    }
}

pub(super) fn focus_vertical(ctx: &mut WmCtx<'_>, direction: VerticalDirection) {
    if ctx.core().model().is_overview_active() {
        crate::overview::focus_direction(ctx, direction.into());
        return;
    }
    let maximized = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_maximized_layout();
    if maximized
        || (!focus_tree_neighbor(ctx, direction.into()) && !direction_focus(ctx, direction.into()))
    {
        focus_stack(ctx, direction.into());
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
