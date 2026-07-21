//! Layout manager — applies computed [`ArrangePlan`]s to backend state.
//!
//! This is the stateful half of the layout system. Pure geometry computation
//! lives in [`algo`]; this module drives the arrange cycle (compute → apply)
//! and handles z-order, monitor sync, and layout switching.

use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::layouts::placement::LayoutPlacement;
use crate::layouts::query::framecount_for_layout;
use crate::layouts::{ArrangePlan, LayoutCommand, LayoutOutput, MonitorUpdates, PresentationMode};
use crate::types::{
    Client, ClientMode, Monitor, MonitorId, PerTagState, Rect, Size, TiledClientInfo, WindowId,
};
use std::cmp::max;
use std::collections::HashMap;

pub fn arrange(ctx: &mut WmCtx<'_>, monitor_id: Option<MonitorId>) {
    if ctx.current_mode().tree_placement().is_some()
        && !ctx
            .current_mode()
            .tree_placement_is_current_for(ctx.core().model())
    {
        ctx.reset_mode();
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
        let resize_hints = globals.config.window.resize_hints;
        let clients = &globals.model.clients;
        let Some(monitor) = globals.model.monitors.get_mut(monitor_id) else {
            return;
        };
        monitor.compute_arrange(clients, &layout_cfg, resize_hints, bar_height, animated)
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

        // 4. In maximized presentation, raise the selected window before animated moves
        //    so it doesn't briefly render beneath siblings during animation.
        if let Some(selected) = ctx
            .core()
            .state()
            .monitor(monitor_id)
            .filter(|m| m.current_layout().is_maximized())
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
        resize_hints: bool,
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
            let moves = match layout {
                PresentationMode::Tiled => compute_manual_tree(
                    self,
                    &layout_clients,
                    layout_cfg,
                    resize_hints,
                    bar_height,
                    animated,
                ),
                PresentationMode::Maximized => {
                    reconcile_manual_tree(self, &layout_clients);
                    crate::layouts::algo::maximized(self, &layout_clients, layout_cfg, animated)
                }
                PresentationMode::Floating => {
                    crate::layouts::algo::floating(self, &layout_clients, animated)
                }
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

fn reconcile_manual_tree(monitor: &mut Monitor, clients: &HashMap<WindowId, Client>) {
    let windows = monitor
        .collect_tiled(clients)
        .into_iter()
        .map(|client| client.win)
        .collect::<Vec<_>>();
    monitor.per_tag_state().layout_tree.reconcile(&windows);
}

fn compute_manual_tree(
    monitor: &mut Monitor,
    clients: &HashMap<WindowId, Client>,
    layout_cfg: &crate::config::config_toml::LayoutConfig,
    resize_hints: bool,
    bar_height: i32,
    animated: bool,
) -> Vec<LayoutOutput> {
    let tiled = monitor.collect_tiled(clients);
    let windows: Vec<_> = tiled.iter().map(|client| client.win).collect();
    let placement = LayoutPlacement::new(
        layout_cfg,
        monitor,
        PresentationMode::Tiled,
        windows.len() as u32,
    );
    let work_rect = placement.work_rect();
    let minimums = tiling_minimum_slots(&placement, &tiled, clients, resize_hints, bar_height);
    let (slots, constraints_fit) = {
        let tree = &mut monitor.per_tag_state().layout_tree;
        tree.reconcile(&windows);
        match tree.constrained_bounds(work_rect, &minimums) {
            Some(slots) => (slots, true),
            None => (tree.bounds(work_rect), false),
        }
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
                options: if resize_hints && constraints_fit {
                    MoveResizeOptions::animate_to(frame_count)
                        .with_size_hints()
                        .with_layout_bounds()
                } else {
                    MoveResizeOptions::animate_to(frame_count)
                },
            })
        })
        .collect()
}

fn tiling_minimum_slots(
    placement: &LayoutPlacement,
    tiled: &[TiledClientInfo],
    clients: &HashMap<WindowId, Client>,
    resize_hints: bool,
    bar_height: i32,
) -> HashMap<WindowId, Size> {
    tiled
        .iter()
        .filter_map(|info| {
            let client = clients.get(&info.win)?;
            let mut size = placement.minimum_slot_size(client, resize_hints);
            let decoration = 2 * client.border_width.max(0) + placement.inner_gap();
            size.w = size.w.max(bar_height.max(1).saturating_add(decoration));
            size.h = size.h.max(bar_height.max(1).saturating_add(decoration));
            Some((client.win, size))
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
    let is_maximized = monitor.current_layout().is_maximized();
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
                border_width_for_layout_client(info, clientcount, is_tiling, is_maximized),
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
    is_maximized: bool,
) -> i32 {
    let strip_border = client.mode.is_true_fullscreen()
        || (client.mode.is_tiling() && ((clientcount == 1 && is_tiling) || is_maximized));

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
        // In maximized presentation, the focused tiled
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

pub fn set_layout(ctx: &mut WmCtx<'_>, layout: super::LayoutCommand) {
    let Some(preset) = layout.tree_preset() else {
        let m = ctx.core_mut().model_mut().expect_selected_monitor_mut();
        m.per_tag_state().layouts.set_layout(layout.presentation());
        finish_layout_change(ctx);
        return;
    };

    ctx.core_mut()
        .model_mut()
        .expect_selected_monitor_mut()
        .per_tag_state()
        .last_tree_layout = preset;
    apply_tree_preset(ctx, preset);
}

pub fn apply_tree_preset(ctx: &mut WmCtx<'_>, preset: crate::layouts::tree::Preset) {
    let (windows, selected, master_count, master_factor) = {
        let monitor = ctx.core().model().expect_selected_monitor();
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
    let monitor = ctx.core_mut().model_mut().expect_selected_monitor_mut();
    let state = monitor.per_tag_state();
    state.layouts.set_layout(PresentationMode::Tiled);
    state
        .layout_tree
        .apply_preset(preset, &windows, selected, master_count, master_factor);
    finish_layout_change(ctx);
}

pub fn focus_tree_neighbor(ctx: &mut WmCtx<'_>, side: crate::layouts::tree::Side) -> bool {
    let neighbor = {
        let monitor = ctx.core().model().expect_selected_monitor();
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
    if !ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_tiling_layout()
    {
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

pub fn resize_tree(ctx: &mut WmCtx<'_>, side: crate::layouts::tree::Side) -> bool {
    if !ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_tiling_layout()
    {
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
    if !ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_tiling_layout()
    {
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
    let tree = &view.monitor.per_tag()?.layout_tree;
    let left = tree.can_resize_side(window, crate::layouts::tree::Side::Left);
    let right = tree.can_resize_side(window, crate::layouts::tree::Side::Right);
    let top = tree.can_resize_side(window, crate::layouts::tree::Side::Top);
    let bottom = tree.can_resize_side(window, crate::layouts::tree::Side::Bottom);
    let horizontal = left || right;
    let vertical = top || bottom;
    if !pointer_tree_resize_allowed(
        view.monitor.current_layout(),
        view.client.mode.is_tiling(),
        tiled_count,
        horizontal,
        vertical,
    ) {
        return None;
    }
    let hit = crate::types::Point::new(point.x - view.client.geo.x, point.y - view.client.geo.y);
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

fn pointer_tree_resize_allowed(
    presentation: PresentationMode,
    client_is_tiled: bool,
    tiled_count: usize,
    horizontal: bool,
    vertical: bool,
) -> bool {
    presentation == PresentationMode::Tiled
        && client_is_tiled
        && tiled_count > 1
        && (horizontal || vertical)
}

fn available_tree_resize_direction(
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

    let (layout_rect, minimum_weight, minimums, monitor_id) = {
        let view = match ctx.core().model().client_view(window) {
            Some(view)
                if view.monitor.current_layout() == PresentationMode::Tiled
                    && view.client.mode.is_tiling()
                    && view.client.is_visible(view.monitor.selected_tags()) =>
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
        let tiled = view.monitor.collect_tiled(&ctx.core().model().clients);
        let minimums = tiling_minimum_slots(
            &placement,
            &tiled,
            &ctx.core().model().clients,
            ctx.core().config().window.resize_hints,
            ctx.core().config().derived.bar_height,
        );
        (
            placement.work_rect(),
            ctx.core().config().layout.minimum_weight,
            minimums,
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
    if candidate
        .constrained_bounds(layout_rect, &minimums)
        .is_none()
    {
        return true;
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

fn selected_tiling_constraints(
    ctx: &WmCtx<'_>,
) -> Option<(LayoutPlacement, HashMap<WindowId, Size>)> {
    let monitor = ctx.core().model().expect_selected_monitor();
    let tiled = monitor.collect_tiled(&ctx.core().model().clients);
    let placement = LayoutPlacement::new(
        &ctx.core().config().layout,
        monitor,
        PresentationMode::Tiled,
        tiled.len() as u32,
    );
    let minimums = tiling_minimum_slots(
        &placement,
        &tiled,
        &ctx.core().model().clients,
        ctx.core().config().window.resize_hints,
        ctx.core().config().derived.bar_height,
    );
    Some((placement, minimums))
}

pub(crate) fn constrained_tree_placement_targets(
    ctx: &WmCtx<'_>,
    source: WindowId,
) -> Vec<crate::layouts::tree::PlacementTarget> {
    let Some((placement, minimums)) = selected_tiling_constraints(ctx) else {
        return Vec::new();
    };
    let Some(tree) = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .per_tag()
        .map(|state| &state.layout_tree)
    else {
        return Vec::new();
    };
    tree.placement_targets(
        source,
        placement.work_rect(),
        ctx.core().config().layout.pointer_edge_fraction,
    )
    .into_iter()
    .filter(|target| {
        let mut candidate = tree.clone();
        candidate.apply_placement_target(source, *target)
            && candidate
                .constrained_bounds(placement.work_rect(), &minimums)
                .is_some()
    })
    .collect()
}

pub(crate) fn preview_constrained_tree_target(
    ctx: &WmCtx<'_>,
    source: WindowId,
    target: crate::layouts::tree::PlacementTarget,
) -> Option<(LayoutPlacement, Rect)> {
    let (placement, minimums) = selected_tiling_constraints(ctx)?;
    let mut candidate = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .per_tag()?
        .layout_tree
        .clone();
    if !candidate.apply_placement_target(source, target) {
        return None;
    }
    let slot = candidate
        .constrained_bounds(placement.work_rect(), &minimums)?
        .get(&source)
        .copied()?;
    Some((placement, slot))
}

pub(crate) fn apply_constrained_tree_target(
    ctx: &mut WmCtx<'_>,
    source: WindowId,
    target: crate::layouts::tree::PlacementTarget,
) -> bool {
    let Some((placement, minimums)) = selected_tiling_constraints(ctx) else {
        return false;
    };
    let Some(mut candidate) = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .per_tag()
        .map(|state| state.layout_tree.clone())
    else {
        return false;
    };
    if !candidate.apply_placement_target(source, target)
        || candidate
            .constrained_bounds(placement.work_rect(), &minimums)
            .is_none()
    {
        return false;
    }
    ctx.core_mut()
        .model_mut()
        .expect_selected_monitor_mut()
        .per_tag_state()
        .layout_tree = candidate;
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
    let Some((placement, minimums)) = selected_tiling_constraints(ctx) else {
        return false;
    };
    let edge_fraction = ctx.core().config().layout.pointer_edge_fraction;
    let Some(mut candidate) = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .per_tag()
        .map(|state| state.layout_tree.clone())
    else {
        return false;
    };
    let changed = candidate.place_at_point(window, point, placement.work_rect(), edge_fraction)
        && candidate
            .constrained_bounds(placement.work_rect(), &minimums)
            .is_some();
    if changed {
        ctx.core_mut()
            .model_mut()
            .expect_selected_monitor_mut()
            .per_tag_state()
            .layout_tree = candidate;
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
    let monitor = ctx.core().model().expect_selected_monitor();
    if !monitor.is_tiling_layout()
        || !ctx
            .core()
            .model()
            .client(window)
            .is_some_and(|client| client.mode.is_tiling())
    {
        return None;
    }
    let tiled = monitor.collect_tiled(&ctx.core().model().clients);
    let placement = LayoutPlacement::new(
        &ctx.core().config().layout,
        monitor,
        PresentationMode::Tiled,
        tiled.len() as u32,
    );
    let minimums = tiling_minimum_slots(
        &placement,
        &tiled,
        &ctx.core().model().clients,
        ctx.core().config().window.resize_hints,
        ctx.core().config().derived.bar_height,
    );
    let mut candidate = monitor.per_tag()?.layout_tree.clone();
    if !candidate.place_at_point(
        window,
        point,
        placement.work_rect(),
        ctx.core().config().layout.pointer_edge_fraction,
    ) {
        return None;
    }
    let slot = candidate
        .constrained_bounds(placement.work_rect(), &minimums)?
        .get(&window)
        .copied()?;
    super::keyboard_placement::tree_slot_outer_rect(ctx, window, placement, slot)
}

pub fn promote_tree(ctx: &mut WmCtx<'_>, window: WindowId) -> bool {
    if !ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_tiling_layout()
    {
        return false;
    }
    let changed = ctx
        .core_mut()
        .model_mut()
        .expect_selected_monitor_mut()
        .per_tag_state()
        .layout_tree
        .promote(window);
    if changed {
        finish_layout_change(ctx);
    }
    changed
}

pub fn toggle_layout(ctx: &mut WmCtx<'_>) {
    let m = ctx.core_mut().model_mut().expect_selected_monitor_mut();
    m.per_tag_state().layouts.toggle_slot();
    finish_layout_change(ctx);
}

/// Toggle maximized-stack presentation without modifying the manual tree.
pub fn toggle_tiling_maximized(ctx: &mut WmCtx<'_>) {
    let next = if ctx
        .core()
        .model()
        .expect_selected_monitor()
        .current_layout()
        == PresentationMode::Maximized
    {
        PresentationMode::Tiled
    } else {
        PresentationMode::Maximized
    };
    let monitor = ctx.core_mut().model_mut().expect_selected_monitor_mut();
    monitor.per_tag_state().layouts.set_layout(next);
    finish_layout_change(ctx);
}

pub(super) fn finish_layout_change(ctx: &mut WmCtx<'_>) {
    let selected_monitor_id = ctx.core().model().selected_monitor_id();
    arrange(ctx, Some(selected_monitor_id));
}

pub fn cycle_layout_direction(ctx: &mut WmCtx<'_>, forward: bool) {
    let current_layout = {
        let monitor = ctx.core().model().expect_selected_monitor();
        match monitor.current_layout() {
            PresentationMode::Floating => LayoutCommand::Floating,
            PresentationMode::Maximized => LayoutCommand::Maximized,
            PresentationMode::Tiled => monitor
                .per_tag()
                .and_then(|state| LayoutCommand::from_tree_preset(state.last_tree_layout))
                .unwrap_or(LayoutCommand::Tile),
        }
    };
    let all_layouts = LayoutCommand::all();
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
        .expect_selected_monitor()
        .tiled_client_count(&ctx.core().model().clients) as i32;
    let m = ctx.core_mut().model_mut().expect_selected_monitor_mut();
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
        .expect_selected_monitor()
        .current_layout()
        .is_tiling();
    if !is_tiling {
        return;
    }

    let current_factor = ctx.core().model().expect_selected_monitor().master_factor;
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
            .expect_selected_monitor()
            .tiled_client_count(&ctx.core().model().clients)
            > 1;
    if animation_on {
        ctx.core_mut().behavior_mut().animated = false;
    }

    let m = ctx.core_mut().model_mut().expect_selected_monitor_mut();
    m.master_factor = new_factor;
    m.per_tag_state().master_factor = new_factor;

    let selected_monitor_id = ctx.core().model().selected_monitor_id();
    arrange(ctx, Some(selected_monitor_id));
    if animation_on {
        ctx.core_mut().behavior_mut().animated = true;
    }
}

#[cfg(test)]
mod tests;
