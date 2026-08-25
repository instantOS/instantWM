use crate::contexts::WmCtx;
use crate::layouts::{LayoutCommand, PresentationMode};
use crate::types::{Monitor, StackDirection, WindowId};

use super::arrange::arrange;

/// Whether keyboard tree commands may edit the manual tree.
///
/// Maximized presentation hides tree geometry behind a uniform stack, so a
/// resize, swap, or promote there would mutate an invisible layout. Pointer
/// interaction enforces the same rule via `manual_tree_interaction_allowed`.
/// Maximized's own order commands (`reorder_maximized_stack`,
/// maximized `swap_bar_titles`) are exempt on purpose: they are its native
/// way of editing the underlying tree.
fn tree_commands_allowed(monitor: &Monitor) -> bool {
    monitor.current_layout() == PresentationMode::Tiled
}

fn tree_preset_changes_allowed(ctx: &WmCtx<'_>) -> bool {
    !ctx.core()
        .interaction()
        .drag
        .active_interaction()
        .is_some_and(|drag| matches!(drag.drag_type(), crate::core_state::DragType::TreeResize(_)))
}

/// Result of trying to reorder the selected maximized-stack entry.
///
/// Keeping the boundary distinct from an inapplicable command lets keyboard
/// policy carry a window to an adjacent tag only after reaching the end of the
/// visible title strip. Floating clients and non-maximized layouts continue to
/// use their ordinary movement behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaximizedStackReorder {
    /// The current presentation or selected client uses another movement model.
    NotApplicable,
    /// Two adjacent title/tree entries were exchanged.
    Reordered,
    /// The selected entry is already at the requested end of the title strip.
    Boundary,
    /// A newly managed client is visible but not yet present in the tree.
    ReconcileRequired,
}

pub fn set_layout(ctx: &mut WmCtx<'_>, layout: LayoutCommand) {
    let Some(preset) = layout.tree_preset() else {
        let monitor = ctx.core_mut().model_mut().expect_selected_monitor_mut();
        monitor.per_tag_state().presentation = layout.presentation();
        finish_layout_change(ctx);
        return;
    };

    // Pressing the key of the layout hidden behind a lens presentation
    // (maximized/floating) lifts the lens and shows the remembered tree.
    // Only a second press, with the layout already visible tiled, resets it.
    let reveal_only = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .per_tag()
        .is_some_and(|state| {
            state.presentation != PresentationMode::Tiled && state.active_preset == preset
        });
    if reveal_only {
        let monitor = ctx.core_mut().model_mut().expect_selected_monitor_mut();
        monitor.per_tag_state().presentation = PresentationMode::Tiled;
        finish_layout_change(ctx);
        return;
    }

    apply_tree_preset(ctx, preset);
}

/// Activate `preset` on the selected monitor's tag.
///
/// Layout slots are remembered per tag: switching to a previously activated
/// layout restores its stored tree (manual edits included), activating a
/// fresh layout seeds its slot from the current tree and then applies the
/// imperative preset rule, and activating the already visible layout reapplies
/// that rule, resetting manual edits to stock geometry.
///
/// Ignored while a pointer tree resize is dragging: its motion events replay
/// an origin snapshot of the tree, so a slot exchange underneath would let
/// the drag clobber the freshly restored slot. Keyboard placement mode is
/// inherently safe — unbound keys cancel it before any command runs.
pub fn apply_tree_preset(ctx: &mut WmCtx<'_>, preset: crate::layouts::tree::Preset) {
    if !tree_preset_changes_allowed(ctx) {
        return;
    }

    let (windows, master_count) = {
        let monitor = ctx.core().model().expect_selected_monitor();
        let windows = monitor
            .collect_tiled(&ctx.core().model().clients)
            .into_iter()
            .map(|client| client.win)
            .collect::<Vec<_>>();
        let requested = monitor.per_tag().map_or(1, |state| state.master_count);
        let requested = if windows.is_empty() {
            requested
        } else {
            requested.min(windows.len())
        };
        (windows, requested)
    };
    let monitor = ctx.core_mut().model_mut().expect_selected_monitor_mut();
    let state = monitor.per_tag_state();
    state.presentation = PresentationMode::Tiled;
    state.master_count = master_count;

    if state.active_preset == preset {
        // Reactivation of the visible layout: rerun the imperative rule. The
        // preset preserves the tree's own leaf order while resetting geometry.
        state
            .layout_tree
            .apply_preset(preset, &windows, master_count);
    } else {
        // Leaving an explicitly chosen slot remembers its tree, manual edits
        // included. The never-activated default tree is not a remembered slot;
        // it simply becomes the seed of the layout being activated.
        if state.preset_activated {
            state
                .stored_trees
                .insert(state.active_preset, state.layout_tree.clone());
        }
        state.active_preset = preset;
        if let Some(stored) = state.stored_trees.remove(&preset) {
            state.layout_tree = stored;
            // The restored slot's force-insertion era ended when it was left.
            state.layout_tree.clear_insertion_provenance();
        } else {
            state
                .layout_tree
                .apply_preset(preset, &windows, master_count);
        }
    }
    state.preset_activated = true;
    finish_layout_change(ctx);
}

pub fn focus_tree_neighbor(ctx: &mut WmCtx<'_>, side: crate::layouts::tree::Side) -> bool {
    let neighbor = {
        let monitor = ctx.core().model().expect_selected_monitor();
        if !tree_commands_allowed(monitor) {
            return false;
        }
        let Some(selected) = monitor.selected else {
            return false;
        };
        monitor
            .per_tag()
            .and_then(|state| state.layout_tree.visual_neighbor(selected, side))
    };
    let Some(neighbor) = neighbor else {
        return false;
    };
    crate::focus::focus(ctx, Some(neighbor));
    true
}

pub fn swap_tree_neighbor(ctx: &mut WmCtx<'_>, side: crate::layouts::tree::Side) -> bool {
    if !tree_commands_allowed(ctx.core().model().expect_selected_monitor()) {
        return false;
    }
    let Some(selected) = ctx.core().model().selected_win() else {
        return false;
    };
    let changed = ctx
        .core_mut()
        .model_mut()
        .expect_selected_monitor_mut()
        .per_tag_state()
        .layout_tree
        .swap_with_neighbor(selected, side)
        .is_some();
    if changed {
        finish_layout_change(ctx);
    }
    changed
}

/// Move the selected tiled client by one entry in maximized title/focus order.
///
/// Maximized presentation deliberately hides the manual tree's geometry. Its
/// exposed spatial model is instead the linear title strip, whose tiled prefix
/// is [`crate::types::Monitor::tiled_tree_order`]. Swap the corresponding leaf
/// occupants so title order, focus traversal, and the tiling restored when
/// leaving maximized presentation all observe the same persistent mutation.
pub fn reorder_maximized_stack(
    ctx: &mut WmCtx<'_>,
    direction: StackDirection,
) -> MaximizedStackReorder {
    let pair = {
        let model = ctx.core().model();
        let monitor = model.expect_selected_monitor();
        if !monitor.is_maximized_layout() {
            return MaximizedStackReorder::NotApplicable;
        }
        let Some(selected) = monitor.selected else {
            return MaximizedStackReorder::NotApplicable;
        };
        let order = monitor.tiled_tree_order(&model.clients);
        let Some(index) = order.iter().position(|&win| win == selected) else {
            return MaximizedStackReorder::NotApplicable;
        };
        let neighbor_index = match direction {
            StackDirection::Previous => index.checked_sub(1),
            StackDirection::Next => (index + 1 < order.len()).then_some(index + 1),
        };
        let Some(neighbor) = neighbor_index.and_then(|index| order.get(index).copied()) else {
            return MaximizedStackReorder::Boundary;
        };
        (selected, neighbor)
    };

    let changed = ctx
        .core_mut()
        .model_mut()
        .expect_selected_monitor_mut()
        .per_tag_state()
        .layout_tree
        .swap_windows(pair.0, pair.1);
    if !changed {
        // `tiled_tree_order` deliberately exposes newly managed tiled clients
        // before the next authoritative arrange has inserted their leaves.
        // Do not reinterpret that transient state as a movement boundary.
        let monitor_id = ctx.core().model().selected_monitor_id();
        ctx.core_mut().queue_layout_for_monitor_urgent(monitor_id);
        return MaximizedStackReorder::ReconcileRequired;
    }

    finish_layout_change(ctx);
    MaximizedStackReorder::Reordered
}

/// Exchange two entries of the bar title strip on `monitor_id`.
///
/// Applies the same policy as [`reorder_maximized_stack`]: in maximized
/// presentation the tiled title prefix is the manual tree, so two tiled
/// entries swap tree leaves; a tiled/floating pair marks the boundary between
/// the tree prefix and the floating tail and cannot cross it. In every other
/// presentation the strip is focus order, so the entries swap positions in
/// the monitor client list.
///
/// Returns `true` when the presented order changed.
pub fn swap_bar_titles(
    ctx: &mut WmCtx<'_>,
    monitor_id: crate::types::MonitorId,
    first: WindowId,
    second: WindowId,
) -> bool {
    let maximized_pair = {
        let Some(model) = ctx.core().model().monitor(monitor_id) else {
            return false;
        };
        let order = model.bar_client_order(&ctx.core().model().clients);
        if !order.contains(&first) || !order.contains(&second) {
            return false;
        }
        model.is_maximized_layout() && {
            let tree_order = model.tiled_tree_order(&ctx.core().model().clients);
            tree_order.contains(&first) && tree_order.contains(&second)
        }
    };

    if maximized_pair {
        let changed = ctx
            .core_mut()
            .model_mut()
            .monitor_mut(monitor_id)
            .expect("validated monitor must remain present")
            .per_tag_state()
            .layout_tree
            .swap_windows(first, second);
        debug_assert!(
            changed,
            "both windows validated against tiled_tree_order must be swappable leaves"
        );
        // The drag may target a monitor other than the selected one; arrange
        // the monitor whose tree actually changed.
        finish_layout_change_for_monitor(ctx, monitor_id);
        return changed;
    }

    ctx.core_mut()
        .model_mut()
        .monitor_mut(monitor_id)
        .expect("validated monitor must remain present")
        .swap_clients_in_stack(first, second)
}

pub fn resize_tree(ctx: &mut WmCtx<'_>, side: crate::layouts::tree::Side) -> bool {
    if !tree_commands_allowed(ctx.core().model().expect_selected_monitor()) {
        return false;
    }
    let Some(selected) = ctx.core().model().selected_win() else {
        return false;
    };
    let layout_config = ctx.core().config().layout;
    let changed = ctx
        .core_mut()
        .model_mut()
        .expect_selected_monitor_mut()
        .per_tag_state()
        .layout_tree
        .resize_with_config(
            selected,
            side,
            crate::layouts::tree::CommandConfig {
                resize_step: layout_config.keyboard_resize_step,
                minimum_weight: layout_config.minimum_weight,
            },
        );
    if changed {
        finish_layout_change(ctx);
    }
    changed
}

pub fn resize_tree_smart(ctx: &mut WmCtx<'_>, grow: bool) -> bool {
    if !tree_commands_allowed(ctx.core().model().expect_selected_monitor()) {
        return false;
    }
    let Some(selected) = ctx.core().model().selected_win() else {
        return false;
    };
    let layout_config = ctx.core().config().layout;
    let changed = ctx
        .core_mut()
        .model_mut()
        .expect_selected_monitor_mut()
        .per_tag_state()
        .layout_tree
        .resize_smart_with_config(
            selected,
            grow,
            crate::layouts::tree::CommandConfig {
                resize_step: layout_config.keyboard_resize_step,
                minimum_weight: layout_config.minimum_weight,
            },
        );
    if changed {
        finish_layout_change(ctx);
    }
    changed
}

pub fn promote_tree(ctx: &mut WmCtx<'_>, window: WindowId) -> bool {
    let eligible = ctx.core().model().client_view(window).is_some_and(|view| {
        view.monitor.id() == ctx.core().model().selected_monitor_id()
            && tree_commands_allowed(view.monitor)
            && view.client.mode().is_normal_tiling()
    });
    if !eligible {
        return false;
    }

    let Some((placement, minimums)) = super::pointer::selected_tiling_constraints(ctx) else {
        return false;
    };
    let candidate_order = {
        let model = ctx.core().model();
        let monitor = model.expect_selected_monitor();
        monitor
            .collect_tiled(&model.clients)
            .into_iter()
            .map(|client| client.win)
            .collect::<Vec<_>>()
    };

    // Raise immediately so the promoted window appears on top while the
    // resulting layout pass is applied.
    ctx.window_backend().raise_window_visual_only(window);
    ctx.window_backend().flush();

    let target_focus = ctx
        .core_mut()
        .model_mut()
        .expect_selected_monitor_mut()
        .per_tag_state()
        .layout_tree
        .promote(window, placement.work_rect(), &minimums, &candidate_order);

    if let Some(target) = target_focus {
        finish_layout_change(ctx);
        if target != window {
            crate::focus::focus(ctx, Some(target));
        }
        true
    } else {
        false
    }
}

/// Toggle maximized-stack presentation without modifying the manual tree.
///
/// When the monitor is in floating layout presentation, this restores manual
/// tiling instead of entering maximized mode.
pub fn toggle_tiling_maximized(ctx: &mut WmCtx<'_>) {
    let next = match ctx
        .core()
        .model()
        .expect_selected_monitor()
        .current_layout()
    {
        PresentationMode::Maximized | PresentationMode::Floating => PresentationMode::Tiled,
        _ => PresentationMode::Maximized,
    };
    let monitor = ctx.core_mut().model_mut().expect_selected_monitor_mut();
    monitor.per_tag_state().presentation = next;
    finish_layout_change(ctx);
}

/// Toggle floating layout presentation without modifying the manual tree or
/// per-window floating state.
pub fn toggle_floating_presentation(ctx: &mut WmCtx<'_>) {
    let next = if ctx
        .core()
        .model()
        .expect_selected_monitor()
        .current_layout()
        == PresentationMode::Floating
    {
        PresentationMode::Tiled
    } else {
        PresentationMode::Floating
    };
    let monitor = ctx.core_mut().model_mut().expect_selected_monitor_mut();
    monitor.per_tag_state().presentation = next;
    finish_layout_change(ctx);
}

pub(crate) fn finish_layout_change(ctx: &mut WmCtx<'_>) {
    let selected_monitor_id = ctx.core().model().selected_monitor_id();
    finish_layout_change_for_monitor(ctx, selected_monitor_id);
}

/// Complete a layout change on `monitor_id`: reconcile tiling invariants,
/// arrange, and re-project presentation signals.
///
/// Callers that target an explicitly chosen monitor (rather than the selected
/// one) must pass it here so the arrange reaches the monitor whose layout
/// actually changed.
pub(crate) fn finish_layout_change_for_monitor(
    ctx: &mut WmCtx<'_>,
    monitor_id: crate::types::MonitorId,
) {
    let is_floating = ctx
        .core()
        .model()
        .monitor(monitor_id)
        .is_some_and(|monitor| monitor.current_layout() == PresentationMode::Floating);
    if !is_floating {
        ctx.core_mut()
            .model_mut()
            .reconcile_client_maximization_for_tiling(monitor_id);
    }
    arrange(ctx, Some(monitor_id));

    // The meaning exposed through the application maximize button changes
    // when the global presentation crosses the tiled/floating boundary, even
    // if a window's geometry does not. Always project the new state.
    let windows = ctx
        .core()
        .model()
        .clients
        .values()
        .filter(|client| client.monitor_id == monitor_id)
        .map(|client| client.win)
        .collect::<Vec<_>>();
    for win in windows {
        crate::client::fullscreen::sync_client_maximized_signal(ctx, win);
    }
}

/// Step to the neighbouring entry of the layout cycle.
///
/// The cycle runs through every command, lenses included. Its position is
/// derived from the full presentation state — a lens entry while lensed, the
/// active slot otherwise — so a single step always lands somewhere visibly
/// different: stepping off a lens reveals or switches the slot, stepping onto
/// one overlays it, and stepping between slots restores remembered trees. A
/// complete lap therefore never reactivates the starting layout, which would
/// reset its manual edits.
pub fn cycle_layout_direction(ctx: &mut WmCtx<'_>, forward: bool) {
    let current_layout = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .current_layout_command();
    let all_layouts = LayoutCommand::all();
    let layouts_len = all_layouts.len();
    let current_idx = all_layouts
        .iter()
        .position(|&layout| layout == current_layout)
        .unwrap_or(0);

    let candidate = if forward {
        (current_idx + 1) % layouts_len
    } else if current_idx == 0 {
        layouts_len - 1
    } else {
        current_idx - 1
    };
    set_layout(ctx, all_layouts[candidate]);
}

/// Re-apply the imperative rule of the active layout slot.
///
/// Explicit reset for the bar indicator's middle button: manual edits return
/// to stock geometry, and a lens presentation is dropped so the result is
/// visible rather than silently rewriting a hidden tree.
pub fn reset_active_layout(ctx: &mut WmCtx<'_>) {
    let preset = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .per_tag()
        .map_or(crate::layouts::tree::Preset::MasterStack, |state| {
            state.active_preset
        });
    apply_tree_preset(ctx, preset);
}

pub fn inc_master_count_by(ctx: &mut WmCtx<'_>, delta: i32) {
    // Applying the new count rebuilds the master-stack preset. Reject the
    // command before touching per-tag state when that rebuild is unsafe.
    if !tree_preset_changes_allowed(ctx) {
        return;
    }
    let window_count = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .tiled_client_count(&ctx.core().model().clients);
    if window_count == 0 || delta == 0 {
        return;
    }
    let state = ctx
        .core_mut()
        .model_mut()
        .expect_selected_monitor_mut()
        .per_tag_state();
    let current = state.master_count.min(window_count);
    let next = shifted_master_count(current, delta, window_count);
    state.master_count = next;
    if next != current {
        apply_tree_preset(ctx, crate::layouts::tree::Preset::MasterStack);
    }
}

pub(super) fn shifted_master_count(current: usize, delta: i32, window_count: usize) -> usize {
    current
        .min(window_count)
        .saturating_add_signed(delta as isize)
        .min(window_count)
}
