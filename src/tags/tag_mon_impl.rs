//! Send the selected client to another monitor.
//!
//! This is a single-function module extracted from the original monolithic
//! `tags.rs`.  It lives under `tags/` because the operation is semantically a
//! tag action (the client's tag membership changes when it crosses monitors),
//! but the heavy lifting — detach/attach, geometry update, z-order sync — is
//! delegated to `monitor::transfer_client`.
//!
//! For floating clients the window is repositioned so that its relative
//! position on the target monitor mirrors its position on the source monitor.
//! Tiled clients are simply detached and re-attached; the layout engine takes
//! care of placement.

use crate::contexts::WmCtx;
use crate::monitor::transfer_client;
use crate::types::{MonitorDirection, MonitorId, WindowId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendToMonitorStrategy {
    FloatingProportional,
    DirectTransfer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SendToMonitorPlan {
    win: WindowId,
    target_id: MonitorId,
    strategy: SendToMonitorStrategy,
}

fn plan_send_to_monitor(
    model: &crate::model::WmModel,
    direction: MonitorDirection,
) -> Option<SendToMonitorPlan> {
    let win = model.selected_win()?;
    if model.monitors.len() <= 1 {
        return None;
    }

    let target_id = crate::types::monitor::find_monitor_by_direction(
        model.monitors.iter(),
        model.selected_monitor_id(),
        direction,
    )?;

    let strategy = if model
        .clients
        .get(&win)
        .is_some_and(|client| client.mode.is_floating())
    {
        SendToMonitorStrategy::FloatingProportional
    } else {
        SendToMonitorStrategy::DirectTransfer
    };

    Some(SendToMonitorPlan {
        win,
        target_id,
        strategy,
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Send the selected client to the monitor in the given direction.
pub fn send_to_monitor(ctx: &mut WmCtx, direction: MonitorDirection) {
    let Some(plan) = plan_send_to_monitor(ctx.core().model(), direction) else {
        return;
    };

    match plan.strategy {
        SendToMonitorStrategy::FloatingProportional => move_floating(ctx, plan.win, plan.target_id),
        SendToMonitorStrategy::DirectTransfer => transfer_client(ctx, plan.win, plan.target_id),
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Move a floating client to `target_id`, preserving its relative position.
fn move_floating(ctx: &mut WmCtx, win: WindowId, target_id: crate::types::MonitorId) {
    // Snapshot source geometry before transfer_client() transfers ownership.
    let (
        client_x,
        client_y,
        src_monitor_x,
        src_monitor_y,
        src_work_area_width,
        src_work_area_height,
    ) = {
        let mon = ctx.core().model().selected_monitor();
        let (monitor_x, monitor_y, work_area_width, work_area_height) = (
            mon.monitor_rect.x,
            mon.monitor_rect.y,
            mon.work_rect.w,
            mon.work_rect.h,
        );

        let (win_x, win_y) = ctx
            .core()
            .state()
            .model
            .clients
            .get(&win)
            .map(|c| (c.geo.x, c.geo.y))
            .unwrap_or((0, 0));

        (
            win_x,
            win_y,
            monitor_x,
            monitor_y,
            work_area_width,
            work_area_height,
        )
    };

    // Fractional position on the source monitor (clamped to avoid division by
    // zero on degenerate monitors).
    let xfact = if src_work_area_width > 0 {
        (client_x - src_monitor_x) as f32 / src_work_area_width as f32
    } else {
        0.0
    };
    let yfact = if src_work_area_height > 0 {
        (client_y - src_monitor_y) as f32 / src_work_area_height as f32
    } else {
        0.0
    };

    // Target monitor geometry.
    let (tgt_monitor_x, tgt_monitor_y, tgt_work_area_width, tgt_work_area_height) = ctx
        .core()
        .state()
        .model
        .monitors
        .get(target_id)
        .map(|m| {
            (
                m.monitor_rect.x,
                m.monitor_rect.y,
                m.work_rect.w,
                m.work_rect.h,
            )
        })
        .unwrap_or((0, 0, 0, 0));

    // Transfer the client to the target monitor.
    {
        transfer_client(ctx, win, target_id);
    }

    // Apply proportional position on the new monitor.
    if let Some(client) = ctx.core_mut().model_mut().clients.get_mut(&win) {
        client.geo.x = tgt_monitor_x + (tgt_work_area_width as f32 * xfact) as i32;
        client.geo.y = tgt_monitor_y + (tgt_work_area_height as f32 * yfact) as i32;
    }

    // Raise so the window is immediately visible on the new monitor. The layout
    // refresh for the affected monitors is handled by `transfer_client`.
    ctx.raise_client(win);
    ctx.window_backend().flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WmModel;
    use crate::types::{Client, ClientMode, Monitor};

    fn model_with_selected_client(mode: ClientMode, monitor_count: usize) -> WmModel {
        let mut model = WmModel::new();
        let mut selected_id = None;

        for _ in 0..monitor_count {
            let id = model.monitors.push(Monitor::default());
            if selected_id.is_none() {
                selected_id = Some(id);
            }
        }

        let Some(selected_id) = selected_id else {
            return model;
        };
        model.monitors.set_selected(selected_id);

        let win = WindowId(42);
        let mut client = Client {
            win,
            monitor_id: selected_id,
            mode,
            ..Client::default()
        };
        client.tags = model.selected_monitor().selected_tags();
        model.clients.insert(win, client);

        if let Some(mon) = model.monitors.get_mut(selected_id) {
            mon.selected = Some(win);
            mon.clients.push(win);
        }

        model
    }

    #[test]
    fn planner_returns_none_when_no_selected_window() {
        let mut model = WmModel::new();
        model.monitors.push(Monitor::default());

        assert_eq!(plan_send_to_monitor(&model, MonitorDirection::NEXT), None);
    }

    #[test]
    fn planner_returns_none_when_only_one_monitor() {
        let model = model_with_selected_client(ClientMode::Tiling, 1);

        assert_eq!(plan_send_to_monitor(&model, MonitorDirection::NEXT), None);
    }

    #[test]
    fn planner_uses_floating_strategy_for_floating_clients() {
        let model = model_with_selected_client(ClientMode::Floating, 2);
        let selected_id = model.selected_monitor_id();
        let target_id = crate::types::monitor::find_monitor_by_direction(
            model.monitors.iter(),
            selected_id,
            MonitorDirection::NEXT,
        )
        .unwrap();

        assert_eq!(
            plan_send_to_monitor(&model, MonitorDirection::NEXT),
            Some(SendToMonitorPlan {
                win: WindowId(42),
                target_id,
                strategy: SendToMonitorStrategy::FloatingProportional,
            })
        );
    }

    #[test]
    fn planner_uses_direct_transfer_for_tiled_clients() {
        let model = model_with_selected_client(ClientMode::Tiling, 2);
        let selected_id = model.selected_monitor_id();
        let target_id = crate::types::monitor::find_monitor_by_direction(
            model.monitors.iter(),
            selected_id,
            MonitorDirection::NEXT,
        )
        .unwrap();

        assert_eq!(
            plan_send_to_monitor(&model, MonitorDirection::NEXT),
            Some(SendToMonitorPlan {
                win: WindowId(42),
                target_id,
                strategy: SendToMonitorStrategy::DirectTransfer,
            })
        );
    }
}
