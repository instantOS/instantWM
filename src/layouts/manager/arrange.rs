use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::layouts::placement::LayoutPlacement;
use crate::layouts::{ArrangePlan, LayoutOutput, PresentationMode};
use crate::types::{Client, Monitor, MonitorId, Size, TiledClientInfo, WindowId};
use std::collections::{BTreeSet, HashMap};

pub fn arrange(ctx: &mut WmCtx<'_>, monitor_id: Option<MonitorId>) {
    // Any authoritative arrange may reconcile the tree, constraints, gaps, or
    // monitor geometry. Pointer placement rebuilds lazily on the next sample.
    ctx.core_mut().state_mut().pointer_placement_cache = None;

    if ctx.current_mode().tree_placement().is_some()
        && !ctx
            .current_mode()
            .tree_placement_is_current_for(ctx.core().model())
    {
        ctx.reset_mode();
    }

    crate::client::apply_visibility(ctx);
    if let Some(id) = monitor_id {
        arrange_monitor(ctx, id);
        super::z_order::sync_monitor_z_order(ctx, id);
    } else {
        let monitor_ids: Vec<MonitorId> = ctx
            .core()
            .model()
            .monitors
            .iter()
            .map(|(id, _)| id)
            .collect();
        for id in monitor_ids {
            arrange_monitor(ctx, id);
            super::z_order::sync_monitor_z_order(ctx, id);
        }
    }

    flush_pending_spawn_animations(ctx, monitor_id);

    ctx.request_space_sync();
    ctx.window_backend().flush();
}

/// Start pending spawn transitions whose assigned monitors were arranged by
/// this pass. Windows on other monitors remain queued until their own layout
/// runs; stale window IDs are discarded.
fn flush_pending_spawn_animations(ctx: &mut WmCtx<'_>, arranged_monitor: Option<MonitorId>) {
    // Drain before running presentation effects so callbacks cannot observe a
    // half-consumed queue. Unrelated monitors are restored before any effect.
    let pending = std::mem::take(&mut ctx.core_mut().pending_work_mut().spawn_animations);
    if pending.is_empty() {
        return;
    }

    let mut ready = Vec::new();
    let mut deferred = BTreeSet::new();
    for win in pending {
        let Some(view) = ctx.core().model().client_view(win) else {
            continue;
        };
        if arranged_monitor.is_none_or(|monitor_id| view.client.monitor_id == monitor_id) {
            ready.push(win);
        } else {
            deferred.insert(win);
        }
    }
    ctx.core_mut()
        .pending_work_mut()
        .spawn_animations
        .extend(deferred);

    for win in ready {
        crate::animation::run_spawn_animation(ctx, win);
    }
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
        // Backend effects deliberately retain their established order.
        for (win, border) in &self.borders {
            ctx.set_border(*win, *border);
        }

        if let Some(monitor) = ctx.core_mut().model_mut().monitor_mut(monitor_id) {
            monitor.bar_height = self.bar_height;
        }

        if let Some(selected) = ctx
            .core()
            .state()
            .monitor(monitor_id)
            .filter(|monitor| monitor.current_layout().is_maximized())
            .and_then(|monitor| monitor.selected)
        {
            ctx.window_backend().raise_window_visual_only(selected);
            ctx.window_backend().flush();
        }

        for output in &self.client_moves {
            ctx.move_resize(output.win, output.rect, output.options);
        }
        for output in &self.fullscreen_moves {
            ctx.move_resize(output.win, output.rect, output.options);
        }

        if let Some(z_order) = &self.z_order {
            ctx.window_backend().apply_z_order(z_order);
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
        self.set_bar_height(bar_height);
        let borders = compute_borders(self, clients);

        // Border and geometry updates form one transaction. Layout against
        // the widths this pass will apply, not the previous client snapshot.
        let layout_clients = clients_with_planned_borders(clients, &borders);

        let is_overview = self.overview_state.is_some();
        let (client_moves, z_order) = if is_overview {
            let overview = crate::overview::compute(self, &layout_clients);
            (overview.moves, Some(overview.z_order))
        } else {
            let moves = match self.current_layout() {
                PresentationMode::Tiled => {
                    compute_manual_tree(self, &layout_clients, layout_cfg, resize_hints, bar_height)
                }
                PresentationMode::Maximized => {
                    reconcile_manual_tree(
                        self,
                        &layout_clients,
                        layout_cfg,
                        resize_hints,
                        bar_height,
                    );
                    crate::layouts::algo::maximized(self, &layout_clients, layout_cfg, animated)
                }
                PresentationMode::Floating => {
                    crate::layouts::algo::floating(self, &layout_clients, animated)
                }
            };
            (moves, None)
        };

        // Fullscreen is an ordinary overview card; reapplying fullscreen
        // geometry there would cover the complete hand.
        let fullscreen_moves = if is_overview {
            Vec::new()
        } else {
            compute_fullscreen_moves(self, clients)
        };

        ArrangePlan {
            bar_height: self.bar_height,
            borders,
            client_moves,
            fullscreen_moves,
            z_order,
        }
    }
}

fn reconcile_manual_tree(
    monitor: &mut Monitor,
    clients: &HashMap<WindowId, Client>,
    layout_cfg: &crate::config::config_toml::LayoutConfig,
    resize_hints: bool,
    bar_height: i32,
) {
    let tiled = monitor.collect_tiling_tree_members(clients);
    let windows = tiled.iter().map(|client| client.win).collect::<Vec<_>>();
    let placement = LayoutPlacement::new(
        layout_cfg,
        monitor,
        PresentationMode::Tiled,
        windows.len() as u32,
    );
    let minimums = tiling_minimum_slots(&placement, &tiled, clients, resize_hints, bar_height);
    monitor.per_tag_state().layout_tree.reconcile_for_layout(
        &windows,
        layout_cfg.new_window_placement,
        placement.work_rect(),
        &minimums,
    );
}

fn compute_manual_tree(
    monitor: &mut Monitor,
    clients: &HashMap<WindowId, Client>,
    layout_cfg: &crate::config::config_toml::LayoutConfig,
    resize_hints: bool,
    bar_height: i32,
) -> Vec<LayoutOutput> {
    let tiled = monitor.collect_tiling_tree_members(clients);
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
        tree.reconcile_for_layout(
            &windows,
            layout_cfg.new_window_placement,
            work_rect,
            &minimums,
        );
        tree.soft_constrained_bounds(work_rect, &minimums)
    };
    tiled
        .into_iter()
        .filter_map(|client| {
            if !clients
                .get(&client.win)
                .is_some_and(|client| client.mode().is_normal_tiling())
            {
                return None;
            }
            let slot = slots.get(&client.win).copied()?;
            Some(LayoutOutput {
                win: client.win,
                rect: placement.client_rect(slot, client.border_width),
                options: if resize_hints && constraints_fit {
                    MoveResizeOptions::animate_to(
                        crate::constants::animation::DEFAULT_ANIMATION_MILLIS,
                    )
                    .with_size_hints()
                    .with_layout_bounds()
                } else {
                    MoveResizeOptions::animate_to(
                        crate::constants::animation::DEFAULT_ANIMATION_MILLIS,
                    )
                },
            })
        })
        .collect()
}

pub(super) fn tiling_minimum_slots(
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

pub(super) fn clients_with_planned_borders(
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
    let client_count = monitor.tiled_client_count(clients) as u32;
    let selected_tags = monitor.visible_tags();

    monitor
        .clients
        .iter()
        .filter_map(|&win| {
            let client = clients.get(&win)?;
            client.is_visible(selected_tags).then(|| {
                (
                    win,
                    border_width_for_layout_client(client, client_count, is_tiling, is_maximized),
                )
            })
        })
        .collect()
}

fn compute_fullscreen_moves(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
) -> Vec<LayoutOutput> {
    let monitor_rect = monitor.monitor_rect;
    let selected_tags = monitor.selected_tags();

    monitor
        .clients
        .iter()
        .filter_map(|&win| {
            let client = clients.get(&win)?;
            (client.mode().is_true_fullscreen() && client.is_visible(selected_tags)).then_some(
                LayoutOutput {
                    win,
                    rect: monitor_rect,
                    options: MoveResizeOptions::immediate(),
                },
            )
        })
        .collect()
}

fn border_width_for_layout_client(
    client: &Client,
    client_count: u32,
    is_tiling: bool,
    is_maximized: bool,
) -> i32 {
    let strip_border = client.mode().is_true_fullscreen()
        || (client.mode().is_normal_tiling() && ((client_count == 1 && is_tiling) || is_maximized));

    if strip_border {
        0
    } else {
        client.old_border_width
    }
}
