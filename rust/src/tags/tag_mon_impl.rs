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

use crate::backend::x11::X11BackendRef;
use crate::backend::BackendRef;
use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
use crate::globals::X11RuntimeConfig;
use crate::layouts::arrange;
use crate::monitor::transfer_client;
use crate::types::{MonitorDirection, Systray, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, StackMode, Window};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Send the selected client to the monitor in the given direction.
pub fn send_to_monitor(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
    systray: Option<&mut Systray>,
    direction: MonitorDirection,
) {
    // -----------------------------------------------------------------------
    // 1. Early-exit guards.
    // -----------------------------------------------------------------------
    let (selected_window, has_multiple_mons) = {
        let sel = core.g.selected_monitor().sel;
        (sel, core.g.monitors.len() > 1)
    };

    let Some(win) = selected_window else { return };
    if !has_multiple_mons {
        return;
    }

    let Some(target_id) = crate::types::monitor::find_monitor_by_direction(
        core.g.monitors.monitors(),
        core.g.selected_monitor_id(),
        direction,
    ) else {
        return;
    };

    // -----------------------------------------------------------------------
    // 2. Dispatch: floating clients get proportional repositioning; tiled
    //    clients just move.
    // -----------------------------------------------------------------------
    let is_floating = core
        .g
        .clients
        .get(&win)
        .map(|c| c.isfloating)
        .unwrap_or(false);

    if is_floating {
        move_floating(core, x11, x11_runtime, systray, win, target_id);
    } else {
        transfer_client(
            &mut WmCtx::X11(WmCtxX11 {
                core: core.reborrow(),
                backend: BackendRef::from_x11(x11.conn, x11.screen_num),
                x11: X11BackendRef::new(x11.conn, x11.screen_num),
                x11_runtime,
                systray,
            }),
            win,
            target_id,
        );
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Move a floating client to `target_id`, preserving its relative position.
fn move_floating(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
    mut systray: Option<&mut Systray>,
    win: WindowId,
    target_id: crate::types::MonitorId,
) {
    // Snapshot source geometry before transfer_client() transfers ownership.
    let (
        client_x,
        client_y,
        src_monitor_x,
        src_monitor_y,
        src_work_area_width,
        src_work_area_height,
    ) = {
        let mon = core.g.selected_monitor();
        let (monitor_x, monitor_y, work_area_width, work_area_height) = (
            mon.monitor_rect.x,
            mon.monitor_rect.y,
            mon.work_rect.w,
            mon.work_rect.h,
        );

        let (win_x, win_y) = core
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
    let (tgt_monitor_x, tgt_monitor_y, tgt_work_area_width, tgt_work_area_height) = core
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
    transfer_client(
        &mut WmCtx::X11(WmCtxX11 {
            core: core.reborrow(),
            backend: BackendRef::from_x11(x11.conn, x11.screen_num),
            x11: X11BackendRef::new(x11.conn, x11.screen_num),
            x11_runtime,
            systray: systray.as_deref_mut(),
        }),
        win,
        target_id,
    );

    // Apply proportional position on the new monitor.
    if let Some(client) = core.g.clients.get_mut(&win) {
        client.geo.x = tgt_monitor_x + (tgt_work_area_width as f32 * xfact) as i32;
        client.geo.y = tgt_monitor_y + (tgt_work_area_height as f32 * yfact) as i32;
    }

    let selmon_id = core.g.selected_monitor_id();
    arrange(
        &mut WmCtx::X11(WmCtxX11 {
            core: core.reborrow(),
            backend: BackendRef::from_x11(x11.conn, x11.screen_num),
            x11: X11BackendRef::new(x11.conn, x11.screen_num),
            x11_runtime,
            systray: systray.as_deref_mut(),
        }),
        Some(selmon_id),
    );

    // Raise so the window is immediately visible on the new monitor.
    let x11_win: Window = win.into();
    let _ = x11.conn.configure_window(
        x11_win,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    );
    let _ = x11.conn.flush();
}
