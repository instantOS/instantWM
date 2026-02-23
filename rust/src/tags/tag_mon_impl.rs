//! Send the selected client to another monitor.
//!
//! This is a single-function module extracted from the original monolithic
//! `tags.rs`.  It lives under `tags/` because the operation is semantically a
//! tag action (the client's tag membership changes when it crosses monitors),
//! but the heavy lifting — detach/attach, geometry update, restack — is
//! delegated to `monitor::send_mon`.
//!
//! For floating clients the window is repositioned so that its relative
//! position on the target monitor mirrors its position on the source monitor.
//! Tiled clients are simply detached and re-attached; the layout engine takes
//! care of placement.

use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::monitor::{arrange, dir_to_mon, send_mon};
use crate::types::Arg;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, StackMode};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Send the selected client to the monitor in direction `arg.i`.
///
/// `arg.i` is passed directly to [`dir_to_mon`] which resolves it to an
/// absolute monitor index.  Positive values mean "next monitor", negative
/// values mean "previous monitor" (matching the dwm convention).
///
/// # Floating client repositioning
///
/// When the client is floating its (x, y) position is converted to a fraction
/// of the *work area* of the source monitor, then mapped onto the work area of
/// the target monitor.  This keeps the window at roughly the same place on
/// screen even when the monitors have different resolutions.
///
/// After the move the window is raised to the top of the stacking order so it
/// is immediately visible.
///
/// # No-ops
/// - No client is currently selected.
/// - Only one monitor is connected.
/// - [`dir_to_mon`] returns `None` (direction is out of range).
pub fn tag_mon(arg: &Arg) {
    // -----------------------------------------------------------------------
    // 1. Early-exit guards.
    // -----------------------------------------------------------------------
    let (sel_win, has_multiple_mons) = {
        let globals = get_globals();
        (
            globals.monitors.get(globals.selmon).and_then(|m| m.sel),
            globals.monitors.len() > 1,
        )
    };

    let Some(win) = sel_win else { return };
    if !has_multiple_mons {
        return;
    }

    let Some(target_id) = dir_to_mon(arg.i) else {
        return;
    };

    // -----------------------------------------------------------------------
    // 2. Dispatch: floating clients get proportional repositioning; tiled
    //    clients just move.
    // -----------------------------------------------------------------------
    let is_floating = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| c.isfloating)
            .unwrap_or(false)
    };

    if is_floating {
        move_floating(win, target_id);
    } else {
        send_mon(win, target_id);
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Move a floating client to `target_id`, preserving its relative position.
fn move_floating(win: x11rb::protocol::xproto::Window, target_id: crate::types::MonitorId) {
    // Snapshot source geometry before send_mon() transfers ownership.
    let (c_x, c_y, src_mx, src_my, src_ww, src_wh) = {
        let globals = get_globals();

        let (mx, my, ww, wh) = globals
            .monitors
            .get(globals.selmon)
            .map(|m| {
                (
                    m.monitor_rect.x,
                    m.monitor_rect.y,
                    m.work_rect.w,
                    m.work_rect.h,
                )
            })
            .unwrap_or((0, 0, 0, 0));

        let (cx, cy) = globals
            .clients
            .get(&win)
            .map(|c| (c.geo.x, c.geo.y))
            .unwrap_or((0, 0));

        (cx, cy, mx, my, ww, wh)
    };

    // Fractional position on the source monitor (clamped to avoid division by
    // zero on degenerate monitors).
    let xfact = if src_ww > 0 {
        (c_x - src_mx) as f32 / src_ww as f32
    } else {
        0.0
    };
    let yfact = if src_wh > 0 {
        (c_y - src_my) as f32 / src_wh as f32
    } else {
        0.0
    };

    // Target monitor geometry.
    let (tgt_mx, tgt_my, tgt_ww, tgt_wh) = {
        let globals = get_globals();
        globals
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
            .unwrap_or((0, 0, 0, 0))
    };

    // Transfer the client to the target monitor.
    send_mon(win, target_id);

    // Apply proportional position on the new monitor.
    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.geo.x = tgt_mx + (tgt_ww as f32 * xfact) as i32;
            client.geo.y = tgt_my + (tgt_wh as f32 * yfact) as i32;
        }
    }

    arrange(Some(get_globals().selmon));

    // Raise so the window is immediately visible on the new monitor.
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        let _ = conn.flush();
    }
}
