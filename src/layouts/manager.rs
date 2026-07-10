//! Layout manager — applies computed [`ArrangePlan`]s to backend state.
//!
//! This is the stateful half of the layout system. Pure geometry computation
//! lives in [`algo`]; this module drives the arrange cycle (compute → apply)
//! and handles z-order, monitor sync, and layout switching.

use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::layouts::{ArrangePlan, LayoutKind, LayoutOutput, MonitorUpdates};
use crate::types::{Client, ClientMode, Monitor, MonitorId, PertagState, WindowId};
use std::cmp::max;
use std::collections::HashMap;

pub fn arrange(ctx: &mut WmCtx<'_>, monitor_id: Option<MonitorId>) {
    crate::mouse::cursor::set_cursor_style(ctx, crate::types::AltCursor::Default);

    if let Some(id) = monitor_id {
        crate::client::apply_visibility(ctx);
        arrange_monitor(ctx, id);
        sync_monitor_z_order(ctx, id);
    } else {
        crate::client::apply_visibility(ctx);

        let mon_indices: Vec<MonitorId> = (0..ctx.core().globals().monitors.count())
            .map(MonitorId)
            .collect();
        for idx in mon_indices {
            arrange_monitor(ctx, idx);
            sync_monitor_z_order(ctx, idx);
        }
    }

    ctx.request_space_sync();
    ctx.backend().flush();
}

pub fn arrange_monitor(ctx: &mut WmCtx<'_>, monitor_id: MonitorId) {
    let bar_height = ctx.core().globals().cfg.bar.height;
    let animated = ctx.core().globals().behavior.animated;
    let layout_cfg = &ctx.core().globals().cfg.layout;

    let clients = ctx.core().globals().clients.map();
    let Some(mut monitor) = ctx.core().globals().monitor(monitor_id).cloned() else {
        return;
    };

    let plan = monitor.compute_arrange(clients, layout_cfg, bar_height, animated);

    plan.apply(ctx, monitor_id);
}

impl ArrangePlan {
    fn apply(self, ctx: &mut WmCtx<'_>, monitor_id: MonitorId) {
        // 1. Save floating geometry for overview mode
        for &win in &self.save_geo {
            if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
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
        if let Some(m) = ctx.core_mut().globals_mut().monitor_mut(monitor_id) {
            m.clientcount = self.monitor_updates.clientcount;
            m.nmaster = self.monitor_updates.nmaster;
            m.mfact = self.monitor_updates.mfact;
            m.work_rect = self.monitor_updates.work_rect;
            m.bar_y = self.monitor_updates.bar_y;
            m.bar_height = self.monitor_updates.bar_height;

            // Sync pertag state back (copy values to avoid borrow conflict)
            let nmaster = m.nmaster;
            let mfact = m.mfact;
            let pertag = m.pertag_state();
            pertag.nmaster = nmaster;
            pertag.mfact = mfact;
        }

        // 4. For monocle, raise the selected window before animated moves
        //    so it doesn't briefly render beneath siblings during animation.
        if let Some(selected) = ctx
            .core()
            .globals()
            .monitor(monitor_id)
            .filter(|m| m.current_layout().is_monocle())
            .and_then(|m| m.sel)
        {
            ctx.backend().raise_window_visual_only(selected);
            ctx.backend().flush();
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
            && let Some(monitor) = ctx.core().globals().monitor(monitor_id)
            && let Some(selected) = monitor.sel
            && self.client_moves.iter().any(|o| o.win == selected)
        {
            ctx.backend().raise_window_visual_only(selected);
            ctx.backend().flush();
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
        let clientcount = self.tiled_client_count(clients) as u32;

        let defaults = PertagState::new(self.show_bar);
        let (nmaster, mfact) = self
            .pertag()
            .map(|p| (p.nmaster, p.mfact))
            .unwrap_or((defaults.nmaster, defaults.mfact));

        self.clientcount = clientcount;
        self.nmaster = nmaster;
        self.mfact = mfact;
        self.update_bar_position(bar_height);

        let bar_y = self.bar_y;
        let work_rect = self.work_rect;

        // Compute borders
        let borders = compute_borders(self, clients);

        // Compute layout moves and save_geo
        let is_overview = self.overview_state.is_some();
        let (client_moves, save_geo) = if is_overview {
            let (moves, save_geo) = crate::overview::compute(self, clients);
            (moves, save_geo)
        } else {
            let layout = self.current_layout();
            (
                layout.compute(self, clients, layout_cfg, animated),
                Vec::new(),
            )
        };

        // Compute fullscreen moves
        let fullscreen_moves = compute_fullscreen_moves(self, clients);

        ArrangePlan {
            monitor_updates: MonitorUpdates {
                clientcount,
                nmaster,
                mfact,
                work_rect,
                bar_y,
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

fn compute_borders(monitor: &Monitor, clients: &HashMap<WindowId, Client>) -> Vec<(WindowId, i32)> {
    let is_tiling = monitor.current_layout().is_tiling();
    let is_monocle = monitor.current_layout().is_monocle();
    let clientcount = monitor.clientcount;
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

    let Some(monitor) = ctx.core().globals().monitor(monitor_id) else {
        return;
    };

    if crate::overview::is_active_on_monitor(ctx.core().globals(), monitor) {
        return;
    }

    let selected_window = match monitor.sel {
        Some(win) => win,
        None => return,
    };
    let layout = monitor.current_layout();
    let is_tiling = layout.is_tiling();

    if !is_tiling {
        ctx.backend().raise_window_visual_only(selected_window);
        ctx.backend().flush();
        return;
    }

    let clients = ctx.core().globals().clients.map();
    let Some(stack) = compute_monitor_z_order(monitor, clients) else {
        return;
    };
    ctx.backend().apply_z_order(&stack);
    ctx.backend().flush();
}

pub(crate) fn compute_monitor_z_order(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
) -> Option<Vec<WindowId>> {
    let selected_window = monitor.sel?;
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
    let m = ctx.core_mut().globals_mut().selected_monitor_mut();
    m.pertag_state().layouts.set_layout(layout);
    finish_layout_change(ctx);
}

pub fn toggle_layout(ctx: &mut WmCtx<'_>) {
    let m = ctx.core_mut().globals_mut().selected_monitor_mut();
    m.pertag_state().layouts.toggle_slot();
    finish_layout_change(ctx);
}

fn finish_layout_change(ctx: &mut WmCtx<'_>) {
    let selected_monitor_id = ctx.core().globals().selected_monitor_id();
    arrange(ctx, Some(selected_monitor_id));
}

pub fn cycle_layout_direction(ctx: &mut WmCtx<'_>, forward: bool) {
    let current_layout = ctx.core().globals().selected_monitor().current_layout();
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

pub fn inc_nmaster_by(ctx: &mut WmCtx<'_>, delta: i32) {
    let ccount = ctx
        .core()
        .globals()
        .selected_monitor()
        .tiled_client_count(ctx.core().globals().clients.map()) as i32;
    let m = ctx.core_mut().globals_mut().selected_monitor_mut();
    if delta > 0 && m.nmaster >= ccount {
        m.nmaster = ccount;
    } else {
        let new_nmaster = max(m.nmaster + delta, 0);
        m.nmaster = new_nmaster;
    }
    m.pertag_state().nmaster = m.nmaster;
    let selected_monitor_id = ctx.core().globals().selected_monitor_id();
    arrange(ctx, Some(selected_monitor_id));
}

pub fn set_mfact(ctx: &mut WmCtx<'_>, mfact_val: f32) {
    if mfact_val == 0.0 {
        return;
    }
    let is_tiling = ctx
        .core()
        .globals()
        .selected_monitor()
        .current_layout()
        .is_tiling();
    if !is_tiling {
        return;
    }

    let current_mfact = ctx.core().globals().selected_monitor().mfact;
    let new_mfact = if mfact_val < 1.0 {
        mfact_val + current_mfact
    } else {
        mfact_val - 1.0
    };
    if !(0.05..=0.95).contains(&new_mfact) {
        return;
    }

    let animation_on = ctx.core().globals().behavior.animated
        && ctx
            .core()
            .globals()
            .selected_monitor()
            .tiled_client_count(ctx.core().globals().clients.map())
            > 1;
    if animation_on {
        ctx.core_mut().globals_mut().behavior.animated = false;
    }

    let m = ctx.core_mut().globals_mut().selected_monitor_mut();
    m.mfact = new_mfact;
    m.pertag_state().mfact = new_mfact;

    let selected_monitor_id = ctx.core().globals().selected_monitor_id();
    arrange(ctx, Some(selected_monitor_id));
    if animation_on {
        ctx.core_mut().globals_mut().behavior.animated = true;
    }
}

#[cfg(test)]
mod tests {
    use super::compute_monitor_z_order;
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
        monitor.sel = Some(selected);
        monitor.bar_win = WindowId(99);
        for &win in order {
            monitor.z_order.attach_top(win);
        }
        monitor
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
