//! Keyboard-driven manual-tree placement session orchestration.

use crate::contexts::WmCtx;
use crate::core_state::{ActiveWmMode, KeyboardPlacementInputs, KeyboardTreePlacement};
use crate::layouts::tree::{LayoutTree, PlacementTarget, Side};
use crate::types::{MonitorId, Rect, WindowId};

use super::manager::{finish_layout_change, selected_tiling};

/// Enter keyboard placement for the selected tiled window. Returns whether
/// placement mode is active afterwards.
pub fn begin_tree_placement(ctx: &mut WmCtx<'_>) -> bool {
    match &ctx.core().state.behavior.current_mode {
        ActiveWmMode::TreePlacement(_) => return true,
        ActiveWmMode::Overview => ctx.reset_mode(),
        ActiveWmMode::Default | ActiveWmMode::Named(_) => {}
    }
    let state = {
        let model = &ctx.core().state.model;
        let monitor = model.expect_selected_monitor();
        let Some(source) = monitor.selected else {
            return false;
        };
        // The selected window belongs to this monitor, so its own map answers
        // the eligibility check without a model-wide scan.
        if monitor.current_layout() != super::PresentationMode::Tiled
            || !monitor
                .client(source)
                .is_some_and(|client| client.mode().is_normal_tiling())
        {
            return false;
        }
        let Some(tree) = monitor.per_tag().map(|state| &state.layout_tree) else {
            return false;
        };
        let tiling = selected_tiling(&ctx.core().state);
        let source_center = tiling
            .slots(tree)
            .0
            .get(&source)
            .map_or_else(|| tiling.work_rect().center(), |rect| rect.center());
        let work_rect = tiling.work_rect();
        let edge_fraction = ctx.core().state.config.layout.pointer_edge_fraction;
        let targets = tree.placement_targets(source, work_rect, edge_fraction, &tiling.minimums);
        let Some(mut state) = KeyboardTreePlacement::new_nearest(
            source,
            monitor.id(),
            monitor.selected_tags(),
            targets,
            source_center,
        ) else {
            return false;
        };
        state.inputs = Some(Box::new(KeyboardPlacementInputs {
            tree: tree.placement_geometry(),
            work_rect,
            minimums: tiling.minimums,
            edge_fraction_bits: edge_fraction.to_bits(),
        }));
        state
    };
    if !ctx.begin_modal_keyboard() {
        return false;
    }
    let Some(preview) = preview_rect(&ctx.core().state, state.source, state.selected_target())
    else {
        ctx.end_modal_keyboard();
        return false;
    };
    ctx.set_current_mode(ActiveWmMode::TreePlacement(state));
    // Placement keys own the pointer until the session ends; a hover-resize
    // offer armed beforehand must give up its cursor and pointer borrow.
    crate::mouse::clear_hover_offer(ctx);
    ctx.update_layout_preview(Some(preview));
    true
}

/// Outer rectangle `source` would occupy after applying `target`.
fn preview_rect(
    state: &crate::core_state::CoreState,
    source: WindowId,
    target: PlacementTarget,
) -> Option<Rect> {
    let tiling = selected_tiling(state);
    // The placement session is armed on the selected monitor, which owns its
    // source: one monitor-scoped lookup feeds both the plan and the projection.
    let monitor = state.model.expect_selected_monitor();
    let plan = monitor.per_tag()?.layout_tree.plan_placement(
        source,
        target,
        tiling.work_rect(),
        &tiling.minimums,
    )?;
    Some(tiling.outer_rect(
        monitor.client(source)?,
        plan.source_slot(),
        state.config.window.resize_hints,
    ))
}

fn refresh_preview(ctx: &mut WmCtx<'_>) {
    let preview = ctx
        .core()
        .state
        .behavior
        .current_mode
        .tree_placement()
        .and_then(|state| preview_rect(&ctx.core().state, state.source, state.selected_target()));
    let animate = ctx
        .core()
        .state
        .config
        .animations
        .keyboard_placement_preview;
    ctx.update_layout_preview_with_animation(preview, animate);
}

/// The active placement session, or `None` after ending one whose
/// monitor/tag/tree context is no longer current.
fn current_placement<'a>(ctx: &'a mut WmCtx<'_>) -> Option<&'a mut KeyboardTreePlacement> {
    if !ctx
        .core()
        .state
        .behavior
        .current_mode
        .tree_placement_is_current_for(&ctx.core().state.model)
    {
        ctx.reset_mode();
        return None;
    }
    ctx.core_mut()
        .state
        .behavior
        .current_mode
        .tree_placement_mut()
}

pub fn step_keyboard_tree_placement(ctx: &mut WmCtx<'_>, side: Side) -> bool {
    if current_placement(ctx).is_some_and(|state| state.select_direction(side)) {
        refresh_preview(ctx);
    }
    true
}

pub fn cycle_keyboard_tree_placement(ctx: &mut WmCtx<'_>, backwards: bool) -> bool {
    if let Some(state) = current_placement(ctx) {
        state.cycle(backwards);
        refresh_preview(ctx);
    }
    true
}

pub fn center_keyboard_tree_placement(ctx: &mut WmCtx<'_>) -> bool {
    if current_placement(ctx).is_some_and(|state| state.select_center_of_current_window()) {
        refresh_preview(ctx);
    }
    true
}

/// Swap the originally armed window with its visual neighbour while keeping
/// keyboard placement active.
pub fn swap_keyboard_tree_placement(ctx: &mut WmCtx<'_>, side: Side) -> bool {
    edit_around_source(ctx, |tree, source| {
        tree.swap_with_neighbor(source, side).is_some()
    })
}

/// Resize the originally armed window while keeping keyboard placement active.
pub fn resize_keyboard_tree_placement(ctx: &mut WmCtx<'_>, side: Side) -> bool {
    let config = (&ctx.core().state.config.layout).into();
    edit_around_source(ctx, |tree, source| tree.resize(source, side, config))
}

fn edit_around_source(
    ctx: &mut WmCtx<'_>,
    edit: impl FnOnce(&mut LayoutTree, WindowId) -> bool,
) -> bool {
    let Some(state) = current_placement(ctx) else {
        return true;
    };
    let source = state.source;
    let tree = &mut ctx
        .core_mut()
        .state
        .model
        .expect_selected_monitor_mut()
        .per_tag_state()
        .layout_tree;
    if edit(tree, source) {
        finish_layout_change(ctx);
    }
    true
}

pub(super) fn refresh_targets_after_arrange(ctx: &mut WmCtx<'_>, arranged: Option<MonitorId>) {
    let state = &ctx.core().state;
    let Some(placement) = state
        .behavior
        .current_mode
        .tree_placement()
        .filter(|placement| arranged.is_none_or(|id| id == placement.monitor_id))
    else {
        return;
    };
    let Some(tree) = state
        .model
        .expect_selected_monitor()
        .per_tag()
        .map(|tag| &tag.layout_tree)
    else {
        finish_keyboard_tree_placement(ctx, false);
        return;
    };
    let tiling = selected_tiling(state);
    let work_rect = tiling.work_rect();
    let edge_fraction = state.config.layout.pointer_edge_fraction;
    let edge_fraction_bits = edge_fraction.to_bits();
    let inputs = placement.inputs.as_deref();
    let same_tree = inputs.is_some_and(|inputs| inputs.tree.matches(tree));
    if same_tree
        && inputs.is_some_and(|inputs| {
            inputs.work_rect == work_rect
                && inputs.minimums == tiling.minimums
                && inputs.edge_fraction_bits == edge_fraction_bits
        })
    {
        // Size increments and aspect hints can change the outline even when
        // the minimum slot sizes and the available targets stay the same.
        refresh_preview(ctx);
        return;
    }

    let preferred = placement.selected_target();
    let targets =
        tree.placement_targets(placement.source, work_rect, edge_fraction, &tiling.minimums);
    // Split indices are only stable while the weighted tree is unchanged.
    // A swap still identifies the same operation by its target window alone.
    let exact = targets.iter().position(|target| {
        (same_tree || preferred.side.is_none())
            && target.target == preferred.target
            && target.side == preferred.side
            && target.candidate_index == preferred.candidate_index
    });
    let inputs = KeyboardPlacementInputs {
        tree: tree.placement_geometry(),
        work_rect,
        minimums: tiling.minimums,
        edge_fraction_bits,
    };
    let placement = ctx
        .core_mut()
        .state
        .behavior
        .current_mode
        .tree_placement_mut()
        .expect("the placement mode is still active");
    let rebuilt = match exact {
        Some(index) => placement.replace_targets(targets, index),
        None => placement.replace_targets_near(targets, preferred.position),
    };
    if rebuilt {
        placement.inputs = Some(Box::new(inputs));
        refresh_preview(ctx);
    } else {
        finish_keyboard_tree_placement(ctx, false);
    }
}

pub fn finish_keyboard_tree_placement(ctx: &mut WmCtx<'_>, apply: bool) -> bool {
    let previous = ctx.transition_current_mode(
        ActiveWmMode::Default,
        crate::overview::ExitMode::RestorePrevious,
    );
    let ActiveWmMode::TreePlacement(state) = previous else {
        return false;
    };
    if !state.is_current_for(&ctx.core().state.model) {
        return true;
    }
    let changed = apply
        && apply_target(
            &mut ctx.core_mut().state,
            state.source,
            state.selected_target(),
        );
    crate::focus::focus(ctx, Some(state.source));
    if changed {
        finish_layout_change(ctx);
    }
    true
}

fn apply_target(
    state: &mut crate::core_state::CoreState,
    source: WindowId,
    target: PlacementTarget,
) -> bool {
    let tiling = selected_tiling(state);
    let tree = &mut state
        .model
        .expect_selected_monitor_mut()
        .per_tag_state()
        .layout_tree;
    let Some(plan) = tree.plan_placement(source, target, tiling.work_rect(), &tiling.minimums)
    else {
        return false;
    };
    *tree = plan.into_tree();
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestWm as Wm;

    use crate::layouts::tree::Preset;
    use crate::test_support::{MonitorBuilder, add_client, add_selected_client};
    use crate::types::{Client, ClientMode, TagMask};

    /// A selected monitor showing `clients` in a master-stack tree, with the
    /// first client selected.
    fn tiled_wm(rect: Rect, clients: Vec<Client>) -> Wm {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        let tags = TagMask::single(1).unwrap();
        let monitor_id = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .rect(rect, rect)
                .tag_count(9)
                .selected_tags(tags)
                .build(),
        );
        wm.core.state.model.monitors.set_selected(monitor_id);
        let windows = clients.iter().map(|client| client.win).collect::<Vec<_>>();
        // Adoption is newest-first, so adding back-to-front leaves the focus
        // list in the order these fixtures were handed in. The first client
        // is the one that ends up selected.
        for (index, client) in clients.into_iter().enumerate().rev() {
            let client = Client {
                tags,
                mode: ClientMode::tiled(),
                ..client
            };
            if index == 0 {
                add_selected_client(&mut wm.core.state.model, monitor_id, client);
            } else {
                add_client(&mut wm.core.state.model, monitor_id, client);
            }
        }
        wm.core
            .state
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .per_tag_state()
            .layout_tree
            .apply_preset(Preset::MasterStack, &windows, 1);
        wm
    }

    fn client(win: u32) -> Client {
        Client {
            win: WindowId(win),
            ..Client::default()
        }
    }

    fn client_with_minimum(win: u32, min_width: i32, min_height: i32) -> Client {
        let mut client = client(win);
        client.size_hints.min_width = min_width;
        client.size_hints.min_height = min_height;
        client
    }

    fn expected_targets(
        state: &crate::core_state::CoreState,
        source: WindowId,
    ) -> Vec<PlacementTarget> {
        let tiling = selected_tiling(state);
        state
            .model
            .expect_selected_monitor()
            .per_tag()
            .unwrap()
            .layout_tree
            .placement_targets(
                source,
                tiling.work_rect(),
                state.config.layout.pointer_edge_fraction,
                &tiling.minimums,
            )
    }

    #[test]
    fn keyboard_placement_navigation_keeps_focus_on_its_source() {
        let mut wm = tiled_wm(Rect::new(0, 0, 1200, 800), vec![client(1), client(2)]);
        let source = WindowId(1);

        assert!(begin_tree_placement(&mut wm.test_ctx()));
        assert_eq!(wm.core.state.model.selected_win(), Some(source));

        assert!(cycle_keyboard_tree_placement(&mut wm.test_ctx(), false));
        assert_eq!(wm.core.state.model.selected_win(), Some(source));

        assert!(step_keyboard_tree_placement(
            &mut wm.test_ctx(),
            Side::Right
        ));
        assert_eq!(wm.core.state.model.selected_win(), Some(source));
    }

    #[test]
    fn keyboard_anchors_follow_the_latest_arranged_geometry() {
        let area = Rect::new(0, 0, 1200, 800);
        let mut wm = tiled_wm(area, vec![client(1), client(2), client(3)]);
        assert!(begin_tree_placement(&mut wm.test_ctx()));
        assert!(center_keyboard_tree_placement(&mut wm.test_ctx()));
        let placement = wm
            .core
            .state
            .behavior
            .current_mode
            .tree_placement()
            .unwrap();
        let previous = placement.selected_target();
        assert_eq!(previous.side, None);
        wm.core
            .state
            .model
            .expect_selected_monitor_mut()
            .available_rect = Rect::new(0, 0, 800, 600);
        finish_layout_change(&mut wm.test_ctx());
        let state = &wm.core.state;
        let current = state.behavior.current_mode.tree_placement().unwrap();
        let selected = current.selected_target();
        assert_eq!(selected.target, previous.target);
        assert_eq!(selected.side, previous.side);
        assert_ne!(selected.position, previous.position);
        assert_eq!(current.targets(), expected_targets(state, current.source));
        assert_eq!(
            state.interaction.layout_preview,
            preview_rect(state, current.source, selected)
        );
    }

    #[test]
    fn arrange_reuses_keyboard_targets_until_placement_inputs_change() {
        let mut wm = tiled_wm(
            Rect::new(0, 0, 1200, 800),
            vec![client(1), client(2), client(3)],
        );
        wm.core.state.config.window.resize_hints = true;
        finish_layout_change(&mut wm.test_ctx());
        assert!(begin_tree_placement(&mut wm.test_ctx()));
        let placement = wm
            .core
            .state
            .behavior
            .current_mode
            .tree_placement()
            .unwrap();
        let cached_targets = placement.targets().as_ptr();
        let source = placement.source;
        let preview_width = wm.core.state.interaction.layout_preview.unwrap().w;

        finish_layout_change(&mut wm.test_ctx());
        let placement = wm
            .core
            .state
            .behavior
            .current_mode
            .tree_placement()
            .unwrap();
        assert_eq!(placement.targets().as_ptr(), cached_targets);

        let monitor = wm.core.state.model.expect_selected_monitor_mut();
        monitor.client_mut(source).unwrap().size_hints.max_width = 120;
        finish_layout_change(&mut wm.test_ctx());
        let state = &wm.core.state;
        let placement = state.behavior.current_mode.tree_placement().unwrap();
        assert_eq!(placement.targets().as_ptr(), cached_targets);
        assert!(state.interaction.layout_preview.unwrap().w < preview_width);

        wm.core.state.config.layout.pointer_edge_fraction = 0.20;
        finish_layout_change(&mut wm.test_ctx());
        let state = &wm.core.state;
        let placement = state.behavior.current_mode.tree_placement().unwrap();
        assert_ne!(placement.targets().as_ptr(), cached_targets);
        assert_eq!(placement.targets(), expected_targets(state, source));
    }

    #[test]
    fn single_tiled_window_has_no_tree_placement_targets() {
        let mut wm = tiled_wm(Rect::new(0, 0, 1200, 800), vec![client(1)]);

        assert!(!begin_tree_placement(&mut wm.test_ctx()));
        assert!(matches!(
            wm.core.state.behavior.current_mode,
            ActiveWmMode::Default
        ));
        assert_eq!(wm.core.state.interaction.layout_preview, None);
    }

    #[test]
    fn keyboard_placement_keeps_targets_when_minimum_sizes_cannot_fit() {
        let mut wm = tiled_wm(
            Rect::new(0, 0, 300, 100),
            vec![
                client_with_minimum(1, 140, 60),
                client_with_minimum(2, 140, 60),
            ],
        );

        let targets = expected_targets(&wm.core.state, WindowId(1));

        assert!(
            targets
                .iter()
                .any(|target| matches!(target.side, Some(Side::Top | Side::Bottom)))
        );
        assert!(begin_tree_placement(&mut wm.test_ctx()));
    }

    #[test]
    fn placement_preview_does_not_expand_beyond_an_overcommitted_slot() {
        let mut client = client_with_minimum(1, 140, 60);
        client.border_width = 0;
        let wm = tiled_wm(Rect::new(0, 0, 100, 100), vec![client]);
        let tiling = selected_tiling(&wm.core.state);
        let slot = Rect::new(10, 20, 10, 8);

        assert_eq!(
            tiling.outer_rect(wm.core.state.model.client(WindowId(1)).unwrap(), slot, true),
            slot
        );
    }
}
