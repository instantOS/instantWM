//! Keyboard-driven manual-tree placement session orchestration.

use crate::contexts::WmCtx;
use crate::layouts::PresentationMode;
use crate::layouts::placement::LayoutPlacement;
use crate::types::{Rect, WindowId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreePlacementStart {
    Started,
    /// The selected window belongs to the manual tiled layout, but that tree
    /// currently has nowhere meaningful to move it.
    NoTargets,
    /// Tree placement was applicable but its backend-owned modal interaction
    /// could not be established safely.
    Unavailable,
    /// The selected window is not eligible for tree placement.
    NotApplicable,
}

pub fn begin_tree_placement(ctx: &mut WmCtx<'_>) -> TreePlacementStart {
    match ctx.current_mode() {
        crate::core_state::ActiveWmMode::TreePlacement(_) => return TreePlacementStart::Started,
        crate::core_state::ActiveWmMode::Overview => ctx.reset_mode(),
        crate::core_state::ActiveWmMode::Default | crate::core_state::ActiveWmMode::Named(_) => {}
    }
    let (source, monitor_id, tags, targets, source_center) = {
        let monitor = ctx.core().model().selected_monitor();
        let Some(source) = monitor.selected else {
            return TreePlacementStart::NotApplicable;
        };
        if !monitor.is_tiling_layout()
            || !ctx
                .core()
                .model()
                .client(source)
                .is_some_and(|client| client.mode.is_tiling())
        {
            return TreePlacementStart::NotApplicable;
        }
        let Some(tree) = monitor.per_tag().map(|state| &state.layout_tree) else {
            return TreePlacementStart::Unavailable;
        };
        let tiled_count = monitor.collect_tiled(&ctx.core().model().clients).len() as u32;
        let layout_rect = LayoutPlacement::new(
            &ctx.core().config().layout,
            monitor,
            PresentationMode::Tiled,
            tiled_count,
        )
        .work_rect();
        let targets = super::manager::constrained_tree_placement_targets(ctx, source);
        let source_center = tree
            .bounds(layout_rect)
            .get(&source)
            .map_or_else(|| layout_rect.center(), |rect| rect.center());
        (
            source,
            monitor.id(),
            monitor.selected_tags(),
            targets,
            source_center,
        )
    };
    if targets.is_empty() {
        return TreePlacementStart::NoTargets;
    }
    let Some(state) = crate::core_state::KeyboardTreePlacement::new_nearest(
        source,
        monitor_id,
        tags,
        targets,
        source_center,
    ) else {
        return TreePlacementStart::Unavailable;
    };
    if !ctx.begin_modal_keyboard() {
        return TreePlacementStart::Unavailable;
    }
    let selected_target = state.selected_target();
    let Some(preview_rect) = tree_placement_preview_rect(ctx, source, selected_target) else {
        ctx.end_modal_keyboard();
        return TreePlacementStart::Unavailable;
    };
    ctx.set_current_mode(crate::core_state::ActiveWmMode::TreePlacement(state));
    ctx.update_layout_preview(Some(preview_rect));
    TreePlacementStart::Started
}

fn tree_placement_preview_rect(
    ctx: &WmCtx<'_>,
    source: WindowId,
    target: crate::layouts::tree::PlacementTarget,
) -> Option<Rect> {
    let (placement, slot) = super::manager::preview_constrained_tree_target(ctx, source, target)?;
    tree_slot_outer_rect(ctx, source, placement, slot)
}

pub(super) fn tree_slot_outer_rect(
    ctx: &WmCtx<'_>,
    source: WindowId,
    placement: LayoutPlacement,
    slot: Rect,
) -> Option<Rect> {
    let client = ctx.core().model().client(source)?;
    let border = client.border_width.max(0);
    let mut content = placement.client_rect(slot, border);
    content.enforce_minimum(
        ctx.core().config().derived.bar_height,
        ctx.core().config().derived.bar_height,
    );
    if ctx.core().config().window.resize_hints {
        let constrained =
            client
                .size_hints
                .constrain_size(content.size(), client.min_aspect, client.max_aspect);
        content.w = constrained.w.min(content.w).max(1);
        content.h = constrained.h.min(content.h).max(1);
    }
    Some(Rect::new(
        content.x,
        content.y,
        content.w + 2 * border,
        content.h + 2 * border,
    ))
}

fn refresh_keyboard_tree_preview(ctx: &mut WmCtx<'_>) {
    let selected = ctx
        .core()
        .behavior()
        .current_mode
        .tree_placement()
        .map(|state| (state.source, state.selected_target()));
    let preview =
        selected.and_then(|(source, target)| tree_placement_preview_rect(ctx, source, target));
    ctx.update_layout_preview(preview);
}

pub fn step_keyboard_tree_placement(ctx: &mut WmCtx<'_>, side: crate::layouts::tree::Side) -> bool {
    if !ctx
        .current_mode()
        .tree_placement_is_current_for(ctx.core().model())
    {
        ctx.reset_mode();
        return true;
    }
    {
        let state = ctx
            .core_mut()
            .behavior_mut()
            .current_mode
            .tree_placement_mut()
            .expect("placement was checked above");
        if !state.select_direction(side) {
            return true;
        }
    }
    refresh_keyboard_tree_preview(ctx);
    true
}

/// Swap the originally armed window with its visual neighbour while keeping
/// keyboard placement active.
pub fn swap_keyboard_tree_placement(ctx: &mut WmCtx<'_>, side: crate::layouts::tree::Side) -> bool {
    if !ctx
        .current_mode()
        .tree_placement_is_current_for(ctx.core().model())
    {
        return finish_keyboard_tree_placement(ctx, false);
    }
    let (source, cursor) = {
        let state = ctx
            .core()
            .behavior()
            .current_mode
            .tree_placement()
            .expect("placement was checked above");
        (state.source, state.selected_target().position)
    };
    let changed = ctx
        .core_mut()
        .model_mut()
        .selected_monitor_mut()
        .per_tag_state()
        .layout_tree
        .swap_with_neighbor(source, side)
        .is_some();
    if changed {
        super::manager::finish_layout_change(ctx);
        rebuild_keyboard_tree_targets(ctx, cursor);
    }
    true
}

/// Resize the originally armed window while keeping keyboard placement active.
pub fn resize_keyboard_tree_placement(
    ctx: &mut WmCtx<'_>,
    side: crate::layouts::tree::Side,
) -> bool {
    if !ctx
        .current_mode()
        .tree_placement_is_current_for(ctx.core().model())
    {
        return finish_keyboard_tree_placement(ctx, false);
    }
    let (source, cursor) = {
        let state = ctx
            .core()
            .behavior()
            .current_mode
            .tree_placement()
            .expect("placement was checked above");
        (state.source, state.selected_target().position)
    };
    let layout_config = ctx.core().config().layout;
    let changed = ctx
        .core_mut()
        .model_mut()
        .selected_monitor_mut()
        .per_tag_state()
        .layout_tree
        .resize_with_config(
            source,
            side,
            crate::layouts::tree::CommandConfig {
                resize_step: layout_config.keyboard_resize_step,
                minimum_weight: layout_config.minimum_weight,
            },
        );
    if changed {
        super::manager::finish_layout_change(ctx);
        rebuild_keyboard_tree_targets(ctx, cursor);
    }
    true
}

fn rebuild_keyboard_tree_targets(ctx: &mut WmCtx<'_>, preferred: crate::types::Point) {
    let targets = {
        let Some(state) = ctx.current_mode().tree_placement() else {
            return;
        };
        super::manager::constrained_tree_placement_targets(ctx, state.source)
    };
    if targets.is_empty() {
        let _ = finish_keyboard_tree_placement(ctx, false);
        return;
    }
    let state = ctx
        .core_mut()
        .behavior_mut()
        .current_mode
        .tree_placement_mut()
        .expect("placement remains active while rebuilding targets");
    let _ = state.replace_targets_near(targets, preferred);
    refresh_keyboard_tree_preview(ctx);
}

pub fn cycle_keyboard_tree_placement(ctx: &mut WmCtx<'_>, backwards: bool) -> bool {
    if !ctx
        .current_mode()
        .tree_placement_is_current_for(ctx.core().model())
    {
        ctx.reset_mode();
        return true;
    }
    {
        let Some(state) = ctx
            .core_mut()
            .behavior_mut()
            .current_mode
            .tree_placement_mut()
        else {
            return false;
        };
        state.cycle(backwards);
    }
    refresh_keyboard_tree_preview(ctx);
    true
}

pub fn center_keyboard_tree_placement(ctx: &mut WmCtx<'_>) -> bool {
    if !ctx
        .current_mode()
        .tree_placement_is_current_for(ctx.core().model())
    {
        ctx.reset_mode();
        return true;
    }
    {
        let Some(state) = ctx
            .core_mut()
            .behavior_mut()
            .current_mode
            .tree_placement_mut()
        else {
            return false;
        };
        if !state.select_center_of_current_window() {
            return true;
        }
    }
    refresh_keyboard_tree_preview(ctx);
    true
}

pub fn finish_keyboard_tree_placement(ctx: &mut WmCtx<'_>, apply: bool) -> bool {
    let previous = ctx.transition_current_mode(
        crate::core_state::ActiveWmMode::Default,
        crate::overview::ExitMode::RestorePrevious,
    );
    let crate::core_state::ActiveWmMode::TreePlacement(state) = previous else {
        return false;
    };
    let context_is_current = state.is_current_for(ctx.core().model());
    let changed = context_is_current
        && apply
        && super::manager::apply_constrained_tree_target(
            ctx,
            state.source,
            state.selected_target(),
        );
    if context_is_current {
        crate::focus::focus(ctx, Some(state.source));
    }
    if changed {
        super::manager::finish_layout_change(ctx);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::layouts::tree::Preset;
    use crate::types::{Client, ClientMode, Monitor, Rect, TagMask, WindowId};
    use crate::wm::Wm;

    #[test]
    fn keyboard_placement_navigation_keeps_focus_on_its_source() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let tags = TagMask::single(1).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1200, 800),
            available_rect: Rect::new(0, 0, 1200, 800),
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        let source = WindowId(1);
        let peer = WindowId(2);
        for win in [source, peer] {
            wm.core.model.insert_client(Client {
                win,
                monitor_id,
                tags,
                mode: ClientMode::Tiling,
                ..Client::default()
            });
        }
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.set_selected_tags(tags);
        monitor.clients = vec![source, peer];
        monitor.selected = Some(source);
        monitor.per_tag_state().layout_tree.apply_preset(
            Preset::MasterStack,
            &[source, peer],
            Some(source),
            1,
            0.5,
        );

        assert_eq!(
            begin_tree_placement(&mut wm.ctx()),
            TreePlacementStart::Started
        );
        assert_eq!(wm.core.model.selected_win(), Some(source));

        assert!(cycle_keyboard_tree_placement(&mut wm.ctx(), false));
        assert_eq!(wm.core.model.selected_win(), Some(source));

        assert!(step_keyboard_tree_placement(
            &mut wm.ctx(),
            crate::layouts::tree::Side::Right,
        ));
        assert_eq!(wm.core.model.selected_win(), Some(source));
    }

    #[test]
    fn single_tiled_window_has_no_tree_placement_targets() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let tags = TagMask::single(1).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1200, 800),
            available_rect: Rect::new(0, 0, 1200, 800),
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        let source = WindowId(1);
        wm.core.model.insert_client(Client {
            win: source,
            monitor_id,
            tags,
            mode: ClientMode::Tiling,
            ..Client::default()
        });
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.set_selected_tags(tags);
        monitor.clients = vec![source];
        monitor.selected = Some(source);
        monitor.per_tag_state().layout_tree.apply_preset(
            Preset::MasterStack,
            &[source],
            Some(source),
            1,
            0.5,
        );

        assert_eq!(
            begin_tree_placement(&mut wm.ctx()),
            TreePlacementStart::NoTargets
        );
        assert!(matches!(
            wm.core.behavior.current_mode,
            crate::core_state::ActiveWmMode::Default
        ));
        assert_eq!(wm.core.layout_preview, None);
    }

    #[test]
    fn keyboard_placement_omits_targets_that_cannot_satisfy_minimum_sizes() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let tags = TagMask::single(1).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 300, 100),
            available_rect: Rect::new(0, 0, 300, 100),
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        let source = WindowId(1);
        let peer = WindowId(2);
        for win in [source, peer] {
            let mut client = Client {
                win,
                monitor_id,
                tags,
                mode: ClientMode::Tiling,
                ..Client::default()
            };
            client.size_hints.minw = 140;
            client.size_hints.minh = 60;
            wm.core.model.insert_client(client);
        }
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.set_selected_tags(tags);
        monitor.clients = vec![source, peer];
        monitor.selected = Some(source);
        monitor.per_tag_state().layout_tree.apply_preset(
            Preset::MasterStack,
            &[source, peer],
            Some(source),
            1,
            0.5,
        );

        let targets =
            crate::layouts::manager::constrained_tree_placement_targets(&wm.ctx(), source);

        assert!(!targets.is_empty());
        assert!(targets.iter().all(|target| !matches!(
            target.side,
            Some(crate::layouts::tree::Side::Top | crate::layouts::tree::Side::Bottom)
        )));
    }
}
