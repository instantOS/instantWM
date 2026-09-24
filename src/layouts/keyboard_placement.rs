//! Keyboard-driven manual-tree placement session orchestration.

use crate::contexts::WmCtx;
use crate::core_state::{ActiveWmMode, KeyboardTreePlacement};
use crate::layouts::tree::{LayoutTree, PlacementTarget, Side};
use crate::types::{Point, Rect, WindowId};

use super::manager::{finish_layout_change, selected_tiling};

/// Enter keyboard placement for the selected tiled window. Returns whether
/// placement mode is active afterwards.
pub fn begin_tree_placement(ctx: &mut WmCtx<'_>) -> bool {
    match ctx.current_mode() {
        ActiveWmMode::TreePlacement(_) => return true,
        ActiveWmMode::Overview => ctx.reset_mode(),
        ActiveWmMode::Default | ActiveWmMode::Named(_) => {}
    }
    let state = {
        let model = ctx.core().model();
        let monitor = model.expect_selected_monitor();
        let Some(source) = monitor.selected else {
            return false;
        };
        if !monitor.is_tiling_layout()
            || !model
                .client(source)
                .is_some_and(|client| client.mode().is_normal_tiling())
        {
            return false;
        }
        let Some(tree) = monitor.per_tag().map(|state| &state.layout_tree) else {
            return false;
        };
        let work_rect = selected_tiling(ctx).work_rect();
        let source_center = tree
            .bounds(work_rect)
            .get(&source)
            .map_or_else(|| work_rect.center(), |rect| rect.center());
        let Some(state) = KeyboardTreePlacement::new_nearest(
            source,
            monitor.id(),
            monitor.selected_tags(),
            placement_targets(ctx, source),
            source_center,
        ) else {
            return false;
        };
        state
    };
    if !ctx.begin_modal_keyboard() {
        return false;
    }
    let Some(preview) = preview_rect(ctx, state.source, state.selected_target()) else {
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

fn placement_targets(ctx: &WmCtx<'_>, source: WindowId) -> Vec<PlacementTarget> {
    let tiling = selected_tiling(ctx);
    ctx.core()
        .model()
        .expect_selected_monitor()
        .per_tag()
        .map(|state| {
            state.layout_tree.placement_targets(
                source,
                tiling.work_rect(),
                ctx.core().config().layout.pointer_edge_fraction,
                &tiling.minimums,
            )
        })
        .unwrap_or_default()
}

/// Outer rectangle `source` would occupy after applying `target`.
fn preview_rect(ctx: &WmCtx<'_>, source: WindowId, target: PlacementTarget) -> Option<Rect> {
    let model = ctx.core().model();
    let tiling = selected_tiling(ctx);
    let plan = model
        .expect_selected_monitor()
        .per_tag()?
        .layout_tree
        .plan_placement(source, target, tiling.work_rect(), &tiling.minimums)?;
    Some(tiling.outer_rect(
        model.client(source)?,
        plan.source_slot(),
        ctx.core().config().window.resize_hints,
    ))
}

fn refresh_preview(ctx: &mut WmCtx<'_>) {
    let preview = ctx
        .current_mode()
        .tree_placement()
        .and_then(|state| preview_rect(ctx, state.source, state.selected_target()));
    ctx.update_layout_preview(preview);
}

/// The active placement session, or `None` after ending one whose
/// monitor/tag/tree context is no longer current.
fn current_placement<'a>(ctx: &'a mut WmCtx<'_>) -> Option<&'a mut KeyboardTreePlacement> {
    if !ctx
        .current_mode()
        .tree_placement_is_current_for(ctx.core().model())
    {
        ctx.reset_mode();
        return None;
    }
    ctx.core_mut()
        .behavior_mut()
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
    let config = (&ctx.core().config().layout).into();
    edit_around_source(ctx, |tree, source| tree.resize(source, side, config))
}

fn edit_around_source(
    ctx: &mut WmCtx<'_>,
    edit: impl FnOnce(&mut LayoutTree, WindowId) -> bool,
) -> bool {
    let Some(state) = current_placement(ctx) else {
        return true;
    };
    let (source, cursor) = (state.source, state.selected_target().position);
    let tree = &mut ctx
        .core_mut()
        .model_mut()
        .expect_selected_monitor_mut()
        .per_tag_state()
        .layout_tree;
    if edit(tree, source) {
        finish_layout_change(ctx);
        rebuild_targets(ctx, cursor);
    }
    true
}

fn rebuild_targets(ctx: &mut WmCtx<'_>, preferred: Point) {
    let Some(source) = ctx
        .current_mode()
        .tree_placement()
        .map(|state| state.source)
    else {
        return;
    };
    let targets = placement_targets(ctx, source);
    let rebuilt = ctx
        .core_mut()
        .behavior_mut()
        .current_mode
        .tree_placement_mut()
        .is_some_and(|state| state.replace_targets_near(targets, preferred));
    if rebuilt {
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
    if !state.is_current_for(ctx.core().model()) {
        return true;
    }
    let changed = apply && apply_target(ctx, state.source, state.selected_target());
    crate::focus::focus(ctx, Some(state.source));
    if changed {
        finish_layout_change(ctx);
    }
    true
}

fn apply_target(ctx: &mut WmCtx<'_>, source: WindowId, target: PlacementTarget) -> bool {
    let tiling = selected_tiling(ctx);
    let tree = &mut ctx
        .core_mut()
        .model_mut()
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
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::layouts::tree::Preset;
    use crate::types::{Client, ClientMode, Monitor, TagMask};
    use crate::wm::Wm;

    /// A selected monitor showing `clients` in a master-stack tree, with the
    /// first client selected.
    fn tiled_wm(rect: Rect, clients: Vec<Client>) -> Wm {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let tags = TagMask::single(1).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: rect,
            available_rect: rect,
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        let windows = clients.iter().map(|client| client.win).collect::<Vec<_>>();
        for client in clients {
            wm.core.model.insert_client(Client {
                monitor_id,
                tags,
                mode: ClientMode::tiled(),
                ..client
            });
        }
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.set_selected_tags(tags);
        monitor.selected = windows.first().copied();
        monitor
            .per_tag_state()
            .layout_tree
            .apply_preset(Preset::MasterStack, &windows, 1);
        monitor.clients = windows;
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

    #[test]
    fn keyboard_placement_navigation_keeps_focus_on_its_source() {
        let mut wm = tiled_wm(Rect::new(0, 0, 1200, 800), vec![client(1), client(2)]);
        let source = WindowId(1);

        assert!(begin_tree_placement(&mut wm.ctx()));
        assert_eq!(wm.core.model.selected_win(), Some(source));

        assert!(cycle_keyboard_tree_placement(&mut wm.ctx(), false));
        assert_eq!(wm.core.model.selected_win(), Some(source));

        assert!(step_keyboard_tree_placement(&mut wm.ctx(), Side::Right));
        assert_eq!(wm.core.model.selected_win(), Some(source));
    }

    #[test]
    fn single_tiled_window_has_no_tree_placement_targets() {
        let mut wm = tiled_wm(Rect::new(0, 0, 1200, 800), vec![client(1)]);

        assert!(!begin_tree_placement(&mut wm.ctx()));
        assert!(matches!(
            wm.core.behavior.current_mode,
            ActiveWmMode::Default
        ));
        assert_eq!(wm.core.interaction.layout_preview, None);
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

        let targets = placement_targets(&wm.ctx(), WindowId(1));

        assert!(
            targets
                .iter()
                .any(|target| matches!(target.side, Some(Side::Top | Side::Bottom)))
        );
        assert!(begin_tree_placement(&mut wm.ctx()));
    }

    #[test]
    fn placement_preview_does_not_expand_beyond_an_overcommitted_slot() {
        let mut client = client_with_minimum(1, 140, 60);
        client.border_width = 0;
        let mut wm = tiled_wm(Rect::new(0, 0, 100, 100), vec![client]);
        let tiling = selected_tiling(&wm.ctx());
        let slot = Rect::new(10, 20, 10, 8);

        assert_eq!(
            tiling.outer_rect(wm.core.model.client(WindowId(1)).unwrap(), slot, true),
            slot
        );
    }
}
