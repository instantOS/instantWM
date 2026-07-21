//! Layout manager — applies computed [`ArrangePlan`]s to backend state.
//!
//! This is the stateful half of the layout system. Pure geometry computation
//! lives in [`algo`]; this module drives the arrange cycle (compute → apply)
//! and handles z-order, monitor sync, and layout switching.

use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::layouts::placement::LayoutPlacement;
use crate::layouts::query::framecount_for_layout;
use crate::layouts::{ArrangePlan, LayoutKind, LayoutOutput, MonitorUpdates};
use crate::types::{Client, ClientMode, Monitor, MonitorId, PerTagState, Rect, WindowId};
use std::cmp::max;
use std::collections::HashMap;

pub fn arrange(ctx: &mut WmCtx<'_>, monitor_id: Option<MonitorId>) {
    if ctx.core().state().tree_placement.is_some() && !keyboard_tree_placement_is_current(ctx) {
        ctx.core_mut().state_mut().tree_placement = None;
        ctx.update_layout_preview(None);
        ctx.end_modal_keyboard();
    }
    crate::mouse::cursor::set_cursor_style(ctx, crate::types::AltCursor::Default);

    if let Some(id) = monitor_id {
        crate::client::apply_visibility(ctx);
        arrange_monitor(ctx, id);
        sync_monitor_z_order(ctx, id);
    } else {
        crate::client::apply_visibility(ctx);

        let mon_indices: Vec<MonitorId> = ctx
            .core()
            .model()
            .monitors
            .iter()
            .map(|(id, _)| id)
            .collect();
        for idx in mon_indices {
            arrange_monitor(ctx, idx);
            sync_monitor_z_order(ctx, idx);
        }
    }

    ctx.request_space_sync();
    ctx.window_backend().flush();
}

pub fn arrange_monitor(ctx: &mut WmCtx<'_>, monitor_id: MonitorId) {
    let plan = {
        let globals = ctx.core_mut().state_mut();
        let bar_height = globals.config.derived.bar_height;
        let animated = globals.behavior.animated;
        let layout_cfg = globals.config.layout;
        let clients = &globals.model.clients;
        let Some(monitor) = globals.model.monitors.get_mut(monitor_id) else {
            return;
        };
        monitor.compute_arrange(clients, &layout_cfg, bar_height, animated)
    };

    plan.apply(ctx, monitor_id);
}

impl ArrangePlan {
    fn apply(self, ctx: &mut WmCtx<'_>, monitor_id: MonitorId) {
        // 1. Save floating geometry for overview mode
        for &win in &self.save_geo {
            if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
                client.save_floating_geometry();
            }
        }

        // 2. Apply border widths
        for (win, border) in &self.borders {
            ctx.set_border(*win, *border);
            if let WmCtx::X11(x11) = ctx {
                x11.x11.set_border_width(*win, *border);
            }
        }

        // 3. Apply monitor updates
        if let Some(m) = ctx.core_mut().model_mut().monitor_mut(monitor_id) {
            m.master_count = self.monitor_updates.master_count;
            m.master_factor = self.monitor_updates.master_factor;
            m.bar_height = self.monitor_updates.bar_height;

            // Sync per-tag state back (copy values to avoid borrow conflict)
            let master_count = m.master_count;
            let master_factor = m.master_factor;
            let pertag = m.per_tag_state();
            pertag.master_count = master_count;
            pertag.master_factor = master_factor;
        }

        // 4. For monocle, raise the selected window before animated moves
        //    so it doesn't briefly render beneath siblings during animation.
        if let Some(selected) = ctx
            .core()
            .state()
            .monitor(monitor_id)
            .filter(|m| m.current_layout().is_monocle())
            .and_then(|m| m.selected)
        {
            ctx.window_backend().raise_window_visual_only(selected);
            ctx.window_backend().flush();
        }

        // 5. Apply client moves (layout placements)
        for output in &self.client_moves {
            ctx.move_resize(output.win, output.rect, output.options);
        }

        // 6. Apply fullscreen moves last — fullscreen overrides layout geometry
        for output in &self.fullscreen_moves {
            ctx.move_resize(output.win, output.rect, output.options);
        }

        // 7. Raise selected window in overview mode
        if self.is_overview
            && let Some(monitor) = ctx.core().model().monitor(monitor_id)
            && let Some(selected) = monitor.selected
            && self.client_moves.iter().any(|o| o.win == selected)
        {
            ctx.window_backend().raise_window_visual_only(selected);
            ctx.window_backend().flush();
        }
    }
}

impl Monitor {
    pub fn compute_arrange(
        &mut self,
        clients: &HashMap<WindowId, Client>,
        layout_cfg: &crate::config::config_toml::LayoutConfig,
        bar_height: i32,
        animated: bool,
    ) -> ArrangePlan {
        let defaults = PerTagState::new(self.show_bar);
        let (master_count, master_factor) = self
            .per_tag()
            .map(|p| (p.master_count, p.master_factor))
            .unwrap_or((defaults.master_count, defaults.master_factor));

        self.master_count = master_count;
        self.master_factor = master_factor;
        self.set_bar_height(bar_height);

        // Compute borders
        let borders = compute_borders(self, clients);

        // Layout geometry and border updates are one transaction. Compute moves
        // from the border widths this arrange pass is about to apply, rather
        // than from the previous pass's client state. Otherwise removing a
        // border (notably when a floating window becomes the sole tiled
        // client) leaves a border-sized strip until the next arrange.
        let layout_clients = clients_with_planned_borders(clients, &borders);

        // Compute layout moves and save_geo
        let is_overview = self.overview_state.is_some();
        let (client_moves, save_geo) = if is_overview {
            let (moves, save_geo) = crate::overview::compute(self, &layout_clients);
            (moves, save_geo)
        } else {
            let layout = self.current_layout();
            let moves = if layout.is_tiling() {
                compute_manual_tree(self, &layout_clients, layout_cfg, animated)
            } else {
                layout.compute(self, &layout_clients, layout_cfg, animated)
            };
            (moves, Vec::new())
        };

        // Compute fullscreen moves
        let fullscreen_moves = compute_fullscreen_moves(self, clients);

        ArrangePlan {
            monitor_updates: MonitorUpdates {
                master_count,
                master_factor,
                bar_height: self.bar_height,
            },
            borders,
            client_moves,
            fullscreen_moves,
            save_geo,
            is_overview,
        }
    }
}

fn compute_manual_tree(
    monitor: &mut Monitor,
    clients: &HashMap<WindowId, Client>,
    layout_cfg: &crate::config::config_toml::LayoutConfig,
    animated: bool,
) -> Vec<LayoutOutput> {
    let tiled = monitor.collect_tiled(clients);
    let windows: Vec<_> = tiled.iter().map(|client| client.win).collect();
    let placement =
        LayoutPlacement::new(layout_cfg, monitor, LayoutKind::Tile, windows.len() as u32);
    let work_rect = placement.work_rect();
    let slots = {
        let tree = &mut monitor.per_tag_state().layout_tree;
        tree.reconcile(&windows);
        tree.bounds(work_rect)
    };
    let frame_count = framecount_for_layout(
        animated,
        windows.len(),
        crate::constants::animation::FAST_ANIM_THRESHOLD,
        crate::constants::animation::FAST_FRAME_COUNT,
        crate::constants::animation::DEFAULT_FRAME_COUNT,
    );

    tiled
        .into_iter()
        .filter_map(|client| {
            let slot = slots.get(&client.win).copied()?;
            Some(LayoutOutput {
                win: client.win,
                rect: placement.client_rect(slot, client.border_width),
                options: MoveResizeOptions::animate_to(frame_count),
            })
        })
        .collect()
}

fn clients_with_planned_borders(
    clients: &HashMap<WindowId, Client>,
    borders: &[(WindowId, i32)],
) -> HashMap<WindowId, Client> {
    let mut planned = clients.clone();
    for &(win, border_width) in borders {
        if let Some(client) = planned.get_mut(&win) {
            client.border_width = border_width;
        }
    }
    planned
}

fn compute_borders(monitor: &Monitor, clients: &HashMap<WindowId, Client>) -> Vec<(WindowId, i32)> {
    let is_tiling = monitor.current_layout().is_tiling();
    let is_monocle = monitor.current_layout().is_monocle();
    let clientcount = monitor.tiled_client_count(clients) as u32;
    let selected_tags = monitor.selected_tags();

    monitor
        .clients
        .iter()
        .filter_map(|&win| {
            let info = clients.get(&win)?;
            let is_visible = info.is_visible(selected_tags);
            if !is_visible {
                return None;
            }

            Some((
                win,
                border_width_for_layout_client(info, clientcount, is_tiling, is_monocle),
            ))
        })
        .collect()
}

fn compute_fullscreen_moves(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
) -> Vec<LayoutOutput> {
    let mon_rect = monitor.monitor_rect;
    let selected_tags = monitor.selected_tags();

    monitor
        .clients
        .iter()
        .filter_map(|&win| {
            let c = clients.get(&win)?;
            if c.mode.is_true_fullscreen() && c.is_visible(selected_tags) {
                Some(LayoutOutput {
                    win,
                    rect: mon_rect,
                    options: MoveResizeOptions::immediate(),
                })
            } else {
                None
            }
        })
        .collect()
}

fn border_width_for_layout_client(
    client: &Client,
    clientcount: u32,
    is_tiling: bool,
    is_monocle: bool,
) -> i32 {
    let strip_border = client.mode.is_true_fullscreen()
        || (client.mode.is_tiling() && ((clientcount == 1 && is_tiling) || is_monocle));

    if strip_border {
        0
    } else {
        client.old_border_width
    }
}

pub fn sync_monitor_z_order(ctx: &mut WmCtx<'_>, monitor_id: MonitorId) {
    ctx.request_bar_update();

    let Some(monitor) = ctx.core().model().monitor(monitor_id) else {
        return;
    };

    if ctx.core().model().is_overview_active_on(monitor) {
        return;
    }

    let selected_window = match monitor.selected {
        Some(win) => win,
        None => return,
    };
    let layout = monitor.current_layout();
    let is_tiling = layout.is_tiling();

    if !is_tiling {
        ctx.window_backend()
            .raise_window_visual_only(selected_window);
        ctx.window_backend().flush();
        return;
    }

    let clients = &ctx.core().model().clients;
    let Some(stack) = compute_monitor_z_order(monitor, clients) else {
        return;
    };
    ctx.window_backend().apply_z_order(&stack);
    ctx.window_backend().flush();
}

pub(crate) fn compute_monitor_z_order(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
) -> Option<Vec<WindowId>> {
    let selected_window = monitor.selected?;
    let selected_tags = monitor.selected_tags();
    let bar_win = monitor.bar_win;
    let tiled_focus = monitor
        .tag_tiled_focus_history
        .get(&selected_tags)
        .copied()
        .filter(|win| {
            clients
                .get(win)
                .is_some_and(|c| c.mode.is_tiling() && c.is_visible(selected_tags))
        });

    let mut tiled_stack = Vec::new();
    let mut floating_stack = Vec::new();
    let mut fullscreen_stack = Vec::new();
    for win in monitor.z_order.iter_bottom_to_top() {
        if let Some(c) = clients.get(&win)
            && c.is_visible(selected_tags)
        {
            match c.mode {
                ClientMode::TrueFullscreen { .. } => fullscreen_stack.push(win),
                ClientMode::Floating | ClientMode::Maximized { .. } => floating_stack.push(win),
                ClientMode::Tiling => tiled_stack.push(win),
                ClientMode::FakeFullscreen { .. } => {}
            }
        }
    }

    let selected_is_fullscreen = fullscreen_stack.contains(&selected_window);
    let selected_is_floating = floating_stack.contains(&selected_window);

    if let Some(tiled_focus) = tiled_focus
        && selected_window != tiled_focus
        && (selected_is_floating || selected_is_fullscreen)
        && let Some(idx) = tiled_stack.iter().position(|&win| win == tiled_focus)
    {
        let selected = tiled_stack.remove(idx);
        tiled_stack.push(selected);
    }

    if let Some(idx) = fullscreen_stack
        .iter()
        .position(|&win| win == selected_window)
    {
        let selected = fullscreen_stack.remove(idx);
        fullscreen_stack.push(selected);
    } else if let Some(idx) = floating_stack
        .iter()
        .position(|&win| win == selected_window)
    {
        let selected = floating_stack.remove(idx);
        floating_stack.push(selected);
    } else {
        // In overlapping tiled layouts such as monocle, the focused tiled
        // client must be projected to the top of the tiled layer without
        // mutating persistent z-order.
        if let Some(idx) = tiled_stack.iter().position(|&win| win == selected_window) {
            let selected = tiled_stack.remove(idx);
            tiled_stack.push(selected);
        }
    }

    // Final z-order: tiled clients, then the bar, then floating clients,
    // and finally fullscreen clients.
    // This keeps every floating window above tiled content while still
    // keeping the selected window topmost within its own class, and guarantees
    // fullscreen windows sit above everything else.
    let mut stack = tiled_stack;
    stack.push(bar_win);
    stack.extend(floating_stack);
    stack.extend(fullscreen_stack);
    Some(stack)
}

pub fn set_layout(ctx: &mut WmCtx<'_>, layout: super::LayoutKind) {
    if layout == LayoutKind::Floating {
        let m = ctx.core_mut().model_mut().selected_monitor_mut();
        m.per_tag_state().layouts.set_layout(layout);
        finish_layout_change(ctx);
        return;
    }

    ctx.core_mut()
        .model_mut()
        .selected_monitor_mut()
        .per_tag_state()
        .last_tree_layout = layout;
    apply_tree_preset(ctx, preset_for_legacy_layout(layout));
}

fn preset_for_legacy_layout(layout: LayoutKind) -> crate::layouts::tree::Preset {
    use crate::layouts::tree::Preset;
    match layout {
        LayoutKind::Tile | LayoutKind::Deck => Preset::MasterStack,
        LayoutKind::Grid | LayoutKind::GaplessGrid => Preset::Grid,
        LayoutKind::HorizGrid => Preset::HorizontalGrid,
        LayoutKind::BottomStack => Preset::BottomStack,
        LayoutKind::BStackHoriz => Preset::BottomStackHorizontal,
        LayoutKind::Monocle => Preset::Focus,
        LayoutKind::Floating => unreachable!("floating is a mode, not a tree preset"),
    }
}

pub fn apply_tree_preset(ctx: &mut WmCtx<'_>, preset: crate::layouts::tree::Preset) {
    let (windows, selected, master_count, master_factor) = {
        let monitor = ctx.core().model().selected_monitor();
        let windows = monitor
            .collect_tiled(&ctx.core().model().clients)
            .into_iter()
            .map(|client| client.win)
            .collect::<Vec<_>>();
        (
            windows,
            monitor.selected,
            monitor.master_count.max(1) as usize,
            f64::from(monitor.master_factor),
        )
    };
    let monitor = ctx.core_mut().model_mut().selected_monitor_mut();
    let state = monitor.per_tag_state();
    state.layouts.set_layout(LayoutKind::Tile);
    state
        .layout_tree
        .apply_preset(preset, &windows, selected, master_count, master_factor);
    finish_layout_change(ctx);
}

pub fn focus_tree_neighbor(ctx: &mut WmCtx<'_>, side: crate::layouts::tree::Side) -> bool {
    let neighbor = {
        let monitor = ctx.core().model().selected_monitor();
        if !monitor.is_tiling_layout() {
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
    if !ctx.core().model().selected_monitor().is_tiling_layout() {
        return false;
    }
    let Some(selected) = ctx.core().model().selected_win() else {
        return false;
    };
    let changed = ctx
        .core_mut()
        .model_mut()
        .selected_monitor_mut()
        .per_tag_state()
        .layout_tree
        .swap_with_neighbor(selected, side)
        .is_some();
    if changed {
        finish_layout_change(ctx);
    }
    changed
}

pub fn resize_tree(ctx: &mut WmCtx<'_>, side: crate::layouts::tree::Side) -> bool {
    if !ctx.core().model().selected_monitor().is_tiling_layout() {
        return false;
    }
    let Some(selected) = ctx.core().model().selected_win() else {
        return false;
    };
    let layout_config = ctx.core().config().layout;
    let changed = ctx
        .core_mut()
        .model_mut()
        .selected_monitor_mut()
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
    if !ctx.core().model().selected_monitor().is_tiling_layout() {
        return false;
    }
    let Some(selected) = ctx.core().model().selected_win() else {
        return false;
    };
    let layout_config = ctx.core().config().layout;
    let changed = ctx
        .core_mut()
        .model_mut()
        .selected_monitor_mut()
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

pub fn place_tree_at_point(
    ctx: &mut WmCtx<'_>,
    window: WindowId,
    point: crate::types::Point,
) -> bool {
    if !ctx.core().model().selected_monitor().is_tiling_layout() {
        return false;
    }
    let layout_rect = {
        let monitor = ctx.core().model().selected_monitor();
        let tiled_count = monitor.collect_tiled(&ctx.core().model().clients).len() as u32;
        LayoutPlacement::new(
            &ctx.core().config().layout,
            monitor,
            LayoutKind::Tile,
            tiled_count,
        )
        .work_rect()
    };
    let edge_fraction = ctx.core().config().layout.pointer_edge_fraction;
    let changed = ctx
        .core_mut()
        .model_mut()
        .selected_monitor_mut()
        .per_tag_state()
        .layout_tree
        .place_at_point(window, point, layout_rect, edge_fraction);
    if changed {
        finish_layout_change(ctx);
    }
    changed
}

/// Compute the exact final outer rectangle for a tiled pointer drop without
/// changing the tree. Returns `None` when the point is not a valid target.
pub fn preview_tree_at_point(
    ctx: &WmCtx<'_>,
    window: WindowId,
    point: crate::types::Point,
) -> Option<Rect> {
    let monitor = ctx.core().model().selected_monitor();
    if !monitor.is_tiling_layout()
        || !ctx
            .core()
            .model()
            .client(window)
            .is_some_and(|client| client.mode.is_tiling())
    {
        return None;
    }
    let tiled_count = monitor.collect_tiled(&ctx.core().model().clients).len() as u32;
    let placement = LayoutPlacement::new(
        &ctx.core().config().layout,
        monitor,
        LayoutKind::Tile,
        tiled_count,
    );
    let slot = monitor.per_tag()?.layout_tree.preview_placement_at_point(
        window,
        point,
        placement.work_rect(),
        ctx.core().config().layout.pointer_edge_fraction,
    )?;
    tree_slot_outer_rect(ctx, window, placement, slot)
}

pub fn promote_tree(ctx: &mut WmCtx<'_>, window: WindowId) -> bool {
    if !ctx.core().model().selected_monitor().is_tiling_layout() {
        return false;
    }
    let changed = ctx
        .core_mut()
        .model_mut()
        .selected_monitor_mut()
        .per_tag_state()
        .layout_tree
        .promote(window);
    if changed {
        finish_layout_change(ctx);
    }
    changed
}

pub fn begin_keyboard_tree_placement(ctx: &mut WmCtx<'_>) -> bool {
    let (source, monitor_id, tags, targets, source_center) = {
        let monitor = ctx.core().model().selected_monitor();
        let Some(source) = monitor.selected else {
            return false;
        };
        if !monitor.is_tiling_layout()
            || !ctx
                .core()
                .model()
                .client(source)
                .is_some_and(|client| client.mode.is_tiling())
        {
            return false;
        }
        let Some(tree) = monitor.per_tag().map(|state| &state.layout_tree) else {
            return false;
        };
        let tiled_count = monitor.collect_tiled(&ctx.core().model().clients).len() as u32;
        let layout_rect = LayoutPlacement::new(
            &ctx.core().config().layout,
            monitor,
            LayoutKind::Tile,
            tiled_count,
        )
        .work_rect();
        let targets = tree.placement_targets(
            source,
            layout_rect,
            ctx.core().config().layout.pointer_edge_fraction,
        );
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
        return false;
    }
    if !ctx.begin_modal_keyboard() {
        return false;
    }
    let selected = targets
        .iter()
        .enumerate()
        .min_by_key(|(_, target)| {
            let dx = i64::from(target.position.x - source_center.x);
            let dy = i64::from(target.position.y - source_center.y);
            dx * dx + dy * dy
        })
        .map_or(0, |(index, _)| index);
    let focus_target = targets[selected].target;
    let cursor_target = targets[selected].position;
    let Some(preview_rect) = tree_placement_preview_rect(ctx, source, targets[selected]) else {
        ctx.end_modal_keyboard();
        return false;
    };
    ctx.core_mut().state_mut().tree_placement = Some(crate::core_state::KeyboardTreePlacement {
        source,
        monitor_id,
        tags,
        targets,
        selected,
    });
    crate::focus::focus(ctx, Some(focus_target));
    ctx.pointer_backend()
        .warp_pointer(f64::from(cursor_target.x), f64::from(cursor_target.y));
    ctx.update_layout_preview(Some(preview_rect));
    true
}

fn tree_placement_preview_rect(
    ctx: &WmCtx<'_>,
    source: WindowId,
    target: crate::layouts::tree::PlacementTarget,
) -> Option<Rect> {
    let monitor = ctx.core().model().selected_monitor();
    let tiled_count = monitor.collect_tiled(&ctx.core().model().clients).len() as u32;
    let placement = LayoutPlacement::new(
        &ctx.core().config().layout,
        monitor,
        LayoutKind::Tile,
        tiled_count,
    );
    let slot = monitor.per_tag()?.layout_tree.preview_placement_target(
        source,
        target,
        placement.work_rect(),
    )?;
    tree_slot_outer_rect(ctx, source, placement, slot)
}

fn tree_slot_outer_rect(
    ctx: &WmCtx<'_>,
    source: WindowId,
    placement: LayoutPlacement,
    slot: Rect,
) -> Option<Rect> {
    let border = ctx.core().model().client(source)?.border_width.max(0);
    let content = placement.client_rect(slot, border);
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
        .state()
        .tree_placement
        .as_ref()
        .map(|state| (state.source, state.targets.get(state.selected).copied()));
    let preview = selected.and_then(|(source, target)| {
        target.and_then(|target| tree_placement_preview_rect(ctx, source, target))
    });
    ctx.update_layout_preview(preview);
}

fn keyboard_tree_placement_is_current(ctx: &WmCtx<'_>) -> bool {
    let Some(state) = ctx.core().state().tree_placement.as_ref() else {
        return false;
    };
    if ctx.core().model().selected_monitor_id() != state.monitor_id {
        return false;
    }
    let monitor = ctx.core().model().selected_monitor();
    monitor.selected_tags() == state.tags
        && ctx
            .core()
            .model()
            .client(state.source)
            .is_some_and(|client| {
                client.monitor_id == state.monitor_id
                    && client.mode.is_tiling()
                    && client.is_visible(state.tags)
            })
        && monitor
            .per_tag()
            .is_some_and(|tag| tag.layout_tree.leaves().contains(&state.source))
}

pub fn step_keyboard_tree_placement(ctx: &mut WmCtx<'_>, side: crate::layouts::tree::Side) -> bool {
    if !keyboard_tree_placement_is_current(ctx) {
        ctx.core_mut().state_mut().tree_placement = None;
        ctx.update_layout_preview(None);
        ctx.end_modal_keyboard();
        return true;
    }
    let next = {
        let Some(state) = ctx.core().state().tree_placement.as_ref() else {
            return false;
        };
        let current = state.targets[state.selected].position;
        state
            .targets
            .iter()
            .enumerate()
            .filter_map(|(index, target)| {
                if index == state.selected {
                    return None;
                }
                let dx = target.position.x - current.x;
                let dy = target.position.y - current.y;
                let primary = match side {
                    crate::layouts::tree::Side::Left => -dx,
                    crate::layouts::tree::Side::Right => dx,
                    crate::layouts::tree::Side::Top => -dy,
                    crate::layouts::tree::Side::Bottom => dy,
                };
                if primary <= 0 {
                    return None;
                }
                let cross = match side {
                    crate::layouts::tree::Side::Left | crate::layouts::tree::Side::Right => {
                        dy.abs()
                    }
                    crate::layouts::tree::Side::Top | crate::layouts::tree::Side::Bottom => {
                        dx.abs()
                    }
                };
                let score = i64::from(primary)
                    + i64::from(cross) * 2
                    + i64::from(cross) * i64::from(cross) / i64::from(primary + 1);
                Some((index, score))
            })
            .min_by_key(|(index, score)| (*score, *index))
            .map(|(index, _)| index)
    };
    let Some(next) = next else { return true };
    let (focus_target, cursor_target) = {
        let state = ctx
            .core_mut()
            .state_mut()
            .tree_placement
            .as_mut()
            .expect("placement was checked above");
        state.selected = next;
        (state.targets[next].target, state.targets[next].position)
    };
    crate::focus::focus(ctx, Some(focus_target));
    ctx.pointer_backend()
        .warp_pointer(f64::from(cursor_target.x), f64::from(cursor_target.y));
    refresh_keyboard_tree_preview(ctx);
    true
}

/// Swap the originally armed window with its visual neighbour while keeping
/// keyboard placement active.
pub fn swap_keyboard_tree_placement(ctx: &mut WmCtx<'_>, side: crate::layouts::tree::Side) -> bool {
    if !keyboard_tree_placement_is_current(ctx) {
        return finish_keyboard_tree_placement(ctx, false);
    }
    let (source, cursor) = {
        let state = ctx
            .core()
            .state()
            .tree_placement
            .as_ref()
            .expect("placement was checked above");
        (state.source, state.targets[state.selected].position)
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
        finish_layout_change(ctx);
        rebuild_keyboard_tree_targets(ctx, cursor);
    }
    true
}

/// Resize the originally armed window while keeping keyboard placement active.
pub fn resize_keyboard_tree_placement(
    ctx: &mut WmCtx<'_>,
    side: crate::layouts::tree::Side,
) -> bool {
    if !keyboard_tree_placement_is_current(ctx) {
        return finish_keyboard_tree_placement(ctx, false);
    }
    let (source, cursor) = {
        let state = ctx
            .core()
            .state()
            .tree_placement
            .as_ref()
            .expect("placement was checked above");
        (state.source, state.targets[state.selected].position)
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
        finish_layout_change(ctx);
        rebuild_keyboard_tree_targets(ctx, cursor);
    }
    true
}

fn rebuild_keyboard_tree_targets(ctx: &mut WmCtx<'_>, preferred: crate::types::Point) {
    let targets = {
        let Some(state) = ctx.core().state().tree_placement.as_ref() else {
            return;
        };
        let monitor = ctx.core().model().selected_monitor();
        let tiled_count = monitor.collect_tiled(&ctx.core().model().clients).len() as u32;
        let layout_rect = LayoutPlacement::new(
            &ctx.core().config().layout,
            monitor,
            LayoutKind::Tile,
            tiled_count,
        )
        .work_rect();
        let targets = monitor.per_tag().map_or_else(Vec::new, |tag| {
            tag.layout_tree.placement_targets(
                state.source,
                layout_rect,
                ctx.core().config().layout.pointer_edge_fraction,
            )
        });
        targets
    };
    if targets.is_empty() {
        let _ = finish_keyboard_tree_placement(ctx, false);
        return;
    }
    let selected = targets
        .iter()
        .enumerate()
        .min_by_key(|(_, target)| {
            let dx = i64::from(target.position.x - preferred.x);
            let dy = i64::from(target.position.y - preferred.y);
            dx * dx + dy * dy
        })
        .map_or(0, |(index, _)| index);
    let focus_target = targets[selected].target;
    let cursor_target = targets[selected].position;
    let state = ctx
        .core_mut()
        .state_mut()
        .tree_placement
        .as_mut()
        .expect("placement remains active while rebuilding targets");
    state.targets = targets;
    state.selected = selected;
    crate::focus::focus(ctx, Some(focus_target));
    ctx.pointer_backend()
        .warp_pointer(f64::from(cursor_target.x), f64::from(cursor_target.y));
    refresh_keyboard_tree_preview(ctx);
}

pub fn cycle_keyboard_tree_placement(ctx: &mut WmCtx<'_>, backwards: bool) -> bool {
    if !keyboard_tree_placement_is_current(ctx) {
        ctx.core_mut().state_mut().tree_placement = None;
        ctx.update_layout_preview(None);
        ctx.end_modal_keyboard();
        return true;
    }
    let (focus_target, cursor_target) = {
        let Some(state) = ctx.core_mut().state_mut().tree_placement.as_mut() else {
            return false;
        };
        let len = state.targets.len();
        state.selected = if backwards {
            (state.selected + len - 1) % len
        } else {
            (state.selected + 1) % len
        };
        (
            state.targets[state.selected].target,
            state.targets[state.selected].position,
        )
    };
    crate::focus::focus(ctx, Some(focus_target));
    ctx.pointer_backend()
        .warp_pointer(f64::from(cursor_target.x), f64::from(cursor_target.y));
    refresh_keyboard_tree_preview(ctx);
    true
}

pub fn center_keyboard_tree_placement(ctx: &mut WmCtx<'_>) -> bool {
    if !keyboard_tree_placement_is_current(ctx) {
        ctx.core_mut().state_mut().tree_placement = None;
        ctx.update_layout_preview(None);
        ctx.end_modal_keyboard();
        return true;
    }
    let (focus_target, cursor_target) = {
        let Some(state) = ctx.core_mut().state_mut().tree_placement.as_mut() else {
            return false;
        };
        let target_window = state.targets[state.selected].target;
        let Some(index) = state
            .targets
            .iter()
            .position(|target| target.target == target_window && target.side.is_none())
        else {
            return true;
        };
        state.selected = index;
        (target_window, state.targets[index].position)
    };
    crate::focus::focus(ctx, Some(focus_target));
    ctx.pointer_backend()
        .warp_pointer(f64::from(cursor_target.x), f64::from(cursor_target.y));
    refresh_keyboard_tree_preview(ctx);
    true
}

pub fn finish_keyboard_tree_placement(ctx: &mut WmCtx<'_>, apply: bool) -> bool {
    let Some(state) = ctx.core_mut().state_mut().tree_placement.take() else {
        return false;
    };
    ctx.update_layout_preview(None);
    ctx.end_modal_keyboard();
    let context_is_current = ctx.core().model().selected_monitor_id() == state.monitor_id
        && ctx.core().model().selected_monitor().selected_tags() == state.tags
        && ctx
            .core()
            .model()
            .selected_monitor()
            .per_tag()
            .is_some_and(|tag| tag.layout_tree.leaves().contains(&state.source));
    let changed = context_is_current
        && apply
        && ctx
            .core_mut()
            .model_mut()
            .selected_monitor_mut()
            .per_tag_state()
            .layout_tree
            .apply_placement_target(state.source, state.targets[state.selected]);
    if context_is_current {
        crate::focus::focus(ctx, Some(state.source));
    }
    if changed {
        finish_layout_change(ctx);
    }
    true
}

pub fn toggle_layout(ctx: &mut WmCtx<'_>) {
    let m = ctx.core_mut().model_mut().selected_monitor_mut();
    m.per_tag_state().layouts.toggle_slot();
    finish_layout_change(ctx);
}

fn finish_layout_change(ctx: &mut WmCtx<'_>) {
    let selected_monitor_id = ctx.core().model().selected_monitor_id();
    arrange(ctx, Some(selected_monitor_id));
}

pub fn cycle_layout_direction(ctx: &mut WmCtx<'_>, forward: bool) {
    let current_layout = {
        let monitor = ctx.core().model().selected_monitor();
        if monitor.current_layout() == LayoutKind::Floating {
            LayoutKind::Floating
        } else {
            monitor
                .per_tag()
                .map(|state| state.last_tree_layout)
                .unwrap_or(LayoutKind::Tile)
        }
    };
    let all_layouts = LayoutKind::all();
    let layouts_len = all_layouts.len();
    let current_idx = all_layouts
        .iter()
        .position(|&x| x == current_layout)
        .unwrap_or(0);

    let candidate = if forward {
        (current_idx + 1) % layouts_len
    } else if current_idx == 0 {
        layouts_len - 1
    } else {
        current_idx - 1
    };
    let final_layout = all_layouts[candidate];
    set_layout(ctx, final_layout);
}

pub fn inc_master_count_by(ctx: &mut WmCtx<'_>, delta: i32) {
    let ccount = ctx
        .core()
        .state()
        .selected_monitor()
        .tiled_client_count(&ctx.core().model().clients) as i32;
    let m = ctx.core_mut().model_mut().selected_monitor_mut();
    if delta > 0 && m.master_count >= ccount {
        m.master_count = ccount;
    } else {
        let new_nmaster = max(m.master_count + delta, 0);
        m.master_count = new_nmaster;
    }
    m.per_tag_state().master_count = m.master_count;
    apply_tree_preset(ctx, crate::layouts::tree::Preset::MasterStack);
}

pub fn set_master_factor(ctx: &mut WmCtx<'_>, delta: f32) {
    if delta == 0.0 {
        return;
    }
    // Kept as a compatibility action name. Under manual tiling this is an
    // imperative local resize, not a persistent parameter which overwrites the
    // tree on every arrange.
    if resize_tree_smart(ctx, delta > 0.0) {
        return;
    }
    let is_tiling = ctx
        .core()
        .state()
        .selected_monitor()
        .current_layout()
        .is_tiling();
    if !is_tiling {
        return;
    }

    let current_factor = ctx.core().model().selected_monitor().master_factor;
    let new_factor = if delta < 1.0 {
        delta + current_factor
    } else {
        delta - 1.0
    };
    if !(0.05..=0.95).contains(&new_factor) {
        return;
    }

    let animation_on = ctx.core().behavior().animated
        && ctx
            .core()
            .state()
            .selected_monitor()
            .tiled_client_count(&ctx.core().model().clients)
            > 1;
    if animation_on {
        ctx.core_mut().behavior_mut().animated = false;
    }

    let m = ctx.core_mut().model_mut().selected_monitor_mut();
    m.master_factor = new_factor;
    m.per_tag_state().master_factor = new_factor;

    let selected_monitor_id = ctx.core().model().selected_monitor_id();
    arrange(ctx, Some(selected_monitor_id));
    if animation_on {
        ctx.core_mut().behavior_mut().animated = true;
    }
}

#[cfg(test)]
mod tests {
    use super::{clients_with_planned_borders, compute_monitor_z_order};
    use crate::config::config_toml::LayoutConfig;
    use crate::layouts::tree::{Preset, Side};
    use crate::types::{Client, Monitor, TagMask, WindowId};
    use std::collections::HashMap;

    fn visible_client(win: WindowId) -> Client {
        let mut client = Client {
            win,
            ..Client::default()
        };
        client.set_tag_mask(TagMask::single(1).unwrap());
        client
    }

    fn monitor_with_order(order: &[WindowId], selected: WindowId) -> Monitor {
        let mut monitor = Monitor::default();
        monitor.set_selected_tags(TagMask::single(1).unwrap());
        monitor.selected = Some(selected);
        monitor.bar_win = WindowId(99);
        for &win in order {
            monitor.z_order.attach_top(win);
        }
        monitor
    }

    #[test]
    fn planned_border_is_used_without_waiting_for_next_arrange() {
        let win = WindowId(1);
        let mut client = visible_client(win);
        client.border_width = 2;
        let clients = HashMap::from([(win, client)]);

        let planned = clients_with_planned_borders(&clients, &[(win, 0)]);

        assert_eq!(planned[&win].border_width, 0);
        assert_eq!(clients[&win].border_width, 2);
    }

    #[test]
    fn projected_z_order_promotes_focused_tiled_without_mutating_persistent_order() {
        let monitor = monitor_with_order(&[WindowId(1), WindowId(2), WindowId(3)], WindowId(2));
        let clients = [WindowId(1), WindowId(2), WindowId(3)]
            .into_iter()
            .map(|win| (win, visible_client(win)))
            .collect::<HashMap<_, _>>();

        let projected = compute_monitor_z_order(&monitor, &clients).unwrap();

        assert_eq!(
            projected,
            vec![WindowId(1), WindowId(3), WindowId(2), WindowId(99)]
        );
        assert_eq!(
            monitor.z_order.iter_bottom_to_top().collect::<Vec<_>>(),
            vec![WindowId(1), WindowId(2), WindowId(3)]
        );
    }

    #[test]
    fn arrange_consumes_persistent_tree_instead_of_reapplying_grid() {
        let mut monitor = monitor_with_order(
            &[WindowId(1), WindowId(2), WindowId(3), WindowId(4)],
            WindowId(1),
        );
        monitor.available_rect = crate::types::Rect::new(0, 0, 100, 100);
        monitor.clients = vec![WindowId(1), WindowId(2), WindowId(3), WindowId(4)];
        let clients = monitor
            .clients
            .iter()
            .copied()
            .map(|window| (window, visible_client(window)))
            .collect::<HashMap<_, _>>();
        let windows = monitor.clients.clone();
        let selected = monitor.selected;
        monitor
            .per_tag_state()
            .layout_tree
            .apply_preset(Preset::Grid, &windows, selected, 1, 0.55);

        let first = monitor.compute_arrange(&clients, &LayoutConfig::default(), 0, false);
        assert!(
            monitor
                .per_tag_state()
                .layout_tree
                .resize(WindowId(1), Side::Right)
        );
        let second = monitor.compute_arrange(&clients, &LayoutConfig::default(), 0, false);

        let first_rect = first
            .client_moves
            .iter()
            .find(|output| output.win == WindowId(1))
            .unwrap()
            .rect;
        let second_rect = second
            .client_moves
            .iter()
            .find(|output| output.win == WindowId(1))
            .unwrap()
            .rect;
        assert_ne!(first_rect, second_rect);
    }

    #[test]
    fn projected_z_order_keeps_floating_above_tiled_and_fullscreen_above_floating() {
        let monitor = monitor_with_order(
            &[WindowId(1), WindowId(2), WindowId(3), WindowId(4)],
            WindowId(2),
        );
        let mut clients = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)]
            .into_iter()
            .map(|win| (win, visible_client(win)))
            .collect::<HashMap<_, _>>();
        clients.get_mut(&WindowId(3)).unwrap().mode = crate::types::ClientMode::Floating;
        let fullscreen = clients.get_mut(&WindowId(4)).unwrap();
        fullscreen.mode = fullscreen.mode.as_fullscreen();

        let projected = compute_monitor_z_order(&monitor, &clients).unwrap();

        assert_eq!(
            projected,
            vec![
                WindowId(1),
                WindowId(2),
                WindowId(99),
                WindowId(3),
                WindowId(4)
            ]
        );
    }

    #[test]
    fn projected_z_order_keeps_last_tiled_focus_visible_under_floating_focus() {
        let mut monitor = monitor_with_order(&[WindowId(1), WindowId(2), WindowId(3)], WindowId(2));
        monitor
            .tag_tiled_focus_history
            .insert(monitor.selected_tags(), WindowId(1));
        let mut clients = [WindowId(1), WindowId(2), WindowId(3)]
            .into_iter()
            .map(|win| (win, visible_client(win)))
            .collect::<HashMap<_, _>>();
        clients.get_mut(&WindowId(2)).unwrap().mode = crate::types::ClientMode::Floating;

        let projected = compute_monitor_z_order(&monitor, &clients).unwrap();

        assert_eq!(
            projected,
            vec![WindowId(3), WindowId(1), WindowId(99), WindowId(2)]
        );
        assert_eq!(
            monitor.z_order.iter_bottom_to_top().collect::<Vec<_>>(),
            vec![WindowId(1), WindowId(2), WindowId(3)]
        );
    }
}
