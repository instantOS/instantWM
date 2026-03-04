//! Send the selected client to another monitor.
//!
//! This is a single-function module extracted from the original monolithic
//! `tags.rs`.  It lives under `tags/` because the operation is semantically a
//! tag action (the client's tag membership changes when it crosses monitors),
//! but the heavy lifting — detach/attach, geometry update, restack — is
//! delegated to `monitor::transfer_client`.
//!
//! For floating clients the window is repositioned so that its relative
//! position on the target monitor mirrors its position on the source monitor.
//! Tiled clients are simply detached and re-attached; the layout engine takes
//! care of placement.

use crate::backend::BackendKind;
use crate::contexts::WmCtx;
use crate::layouts::arrange;
use crate::monitor::transfer_client;
use crate::types::{MonitorDirection, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, StackMode, Window};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Send the selected client to the monitor in the given direction.
pub fn send_to_monitor(ctx: &mut WmCtx, direction: MonitorDirection) {
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    // -----------------------------------------------------------------------
    // 1. Early-exit guards.
    // -----------------------------------------------------------------------
    let (sel_win, has_multiple_mons) = {
        let sel = ctx.g.selmon().and_then(|mon| mon.sel);
        (sel, ctx.g.monitors.len() > 1)
    };

    let Some(win) = sel_win else { return };
    if !has_multiple_mons {
        return;
    }

    let Some(target_id) = crate::types::monitor::find_monitor_by_direction(
        ctx.g.monitors.monitors(),
        ctx.g.selmon_id(),
        direction,
    ) else {
        return;
    };

    // -----------------------------------------------------------------------
    // 2. Dispatch: floating clients get proportional repositioning; tiled
    //    clients just move.
    // -----------------------------------------------------------------------
    let is_floating = ctx
        .g
        .clients
        .get(&win)
        .map(|c| c.isfloating)
        .unwrap_or(false);

    if is_floating {
        move_floating(ctx, win, target_id);
    } else {
        transfer_client(ctx, win, target_id);
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Move a floating client to `target_id`, preserving its relative position.
fn move_floating(ctx: &mut WmCtx, win: WindowId, target_id: crate::types::MonitorId) {
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    // Snapshot source geometry before transfer_client() transfers ownership.
    let (
        client_x,
        client_y,
        src_monitor_x,
        src_monitor_y,
        src_work_area_width,
        src_work_area_height,
    ) = {
        let (monitor_x, monitor_y, work_area_width, work_area_height) = ctx
            .g
            .selmon()
            .map(|m| {
                (
                    m.monitor_rect.x,
                    m.monitor_rect.y,
                    m.work_rect.w,
                    m.work_rect.h,
                )
            })
            .unwrap_or((0, 0, 0, 0));

        let (win_x, win_y) = ctx
            .g
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
        .g
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
    transfer_client(ctx, win, target_id);

    // Apply proportional position on the new monitor.
    if let Some(client) = ctx.g.clients.get_mut(&win) {
        client.geo.x = tgt_monitor_x + (tgt_work_area_width as f32 * xfact) as i32;
        client.geo.y = tgt_monitor_y + (tgt_work_area_height as f32 * yfact) as i32;
    }

    arrange(ctx, Some(ctx.g.selmon_id()));

    // Raise so the window is immediately visible on the new monitor.
    if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
        let x11_win: Window = win.into();
        let _ = conn.configure_window(
            x11_win,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
        let _ = conn.flush();
    }
}
