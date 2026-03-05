//! Batch operations across multiple floating windows.
//!
//! These functions operate on all floating clients of a monitor at once, as
//! opposed to the per-window helpers in the other sub-modules.
//!
//! # Functions
//!
//! | Function               | Purpose                                                  |
//! |------------------------|----------------------------------------------------------|
//! | `save_all_floating`    | Snapshot geometry of every non-snapped floating client   |
//! | `restore_all_floating` | Restore geometry of every non-snapped floating client    |
//! | `distribute_clients`   | Arrange all visible floating windows in an even grid     |
//!
//! `save_all_floating` / `restore_all_floating` are called around overview
//! mode (see `tags.rs`) so that window positions survive the overview layout
//! and are correctly restored when the user switches back.

use crate::client::resize;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::types::*;

// ── Save / restore all floating ───────────────────────────────────────────────

/// Snapshot the geometry of every non-snapped floating client on `monitor_id`.
///
/// Only clients whose tag belongs to a tag that currently has **no** tiling
/// layout (i.e. a pure floating tag) are included.  Snapped windows are
/// excluded because their geometry is managed by the snap system, not by free
/// floating.
///
/// Pair with [`restore_all_floating`] to round-trip positions across a layout
/// change (e.g. entering / leaving overview mode).
pub fn save_all_floating(ctx: &mut WmCtx, monitor_id: Option<usize>) {
    let Some(mid) = monitor_id else { return };

    let wins_to_save = collect_floating_wins(ctx.g, mid);
    for win in wins_to_save {
        super::state::save_floating_win(&mut ctx.core, win);
    }
}

/// Restore the geometry of every non-snapped floating client on `monitor_id`.
///
/// Counterpart to [`save_all_floating`]: resizes each window back to the rect
/// that was captured by the most recent `save_all_floating` call.
pub fn restore_all_floating(ctx: &mut WmCtx, monitor_id: Option<usize>) {
    let Some(mid) = monitor_id else { return };

    let wins_to_restore = collect_floating_wins(ctx.g, mid);
    for win in wins_to_restore {
        super::state::restore_floating_win(ctx, win);
    }
}

/// Walk `monitor_id`'s client list and return all windows that are:
/// - on a tag that has no tiling layout active, and
/// - not currently snapped.
///
/// This is the shared selection logic for both save and restore.
fn collect_floating_wins(globals: &crate::globals::Globals, mid: usize) -> Vec<WindowId> {
    let Some(mon) = globals.monitor(mid) else {
        return Vec::new();
    };

    let numtags = mon.tags.len();
    let mut wins = Vec::new();

    for tag_idx in 0..numtags {
        // Skip tags that have a tiling layout — only purely-floating tags matter.
        let tag_is_floating = match mon.tags.get(tag_idx) {
            Some(tag) => !tag.layouts.is_tiling(),
            _ => false,
        };

        if !tag_is_floating {
            continue;
        }

        let tag_mask = 1u32 << tag_idx;
        for (c_win, c) in mon.iter_clients(globals.clients.map()) {
            if (c.tags & tag_mask) != 0 && c.snap_status == SnapPosition::None {
                wins.push(c_win);
            }
        }
    }

    wins
}

// ── Distribute ────────────────────────────────────────────────────────────────

/// Tile all visible, non-fixed, non-snapped floating windows on the selected
/// monitor into an evenly-spaced grid.
///
/// The grid dimensions are chosen so that the number of columns is the ceiling
/// of `sqrt(n)` (giving a roughly square layout), and rows are computed from
/// that.  Each cell receives one window, sized to exactly fill its cell.
///
/// Does nothing when there are no qualifying windows.
pub fn distribute_clients(ctx: &mut WmCtxX11) {
    let sel_mon_id = ctx.core.g.selected_monitor_id();

    let (floating_wins, work_rect) = collect_distribute_targets(ctx.core.g, sel_mon_id);

    if floating_wins.is_empty() {
        return;
    }

    let n = floating_wins.len();

    // Choose a roughly-square grid.
    let cols = (n as f32).sqrt().ceil() as i32;
    let rows = ((n as f32) / (cols as f32)).ceil() as i32;

    let cell_w = work_rect.w / cols;
    let cell_h = work_rect.h / rows;

    let mut wm_ctx = crate::contexts::WmCtx::X11(crate::contexts::WmCtxX11 {
        core: &mut ctx.core,
        backend: ctx.backend,
        x11: ctx.x11,
    });
    for (i, win) in floating_wins.into_iter().enumerate() {
        let col = (i as i32) % cols;
        let row = (i as i32) / cols;

        resize(
            &mut wm_ctx,
            win,
            &Rect {
                x: work_rect.x + col * cell_w,
                y: work_rect.y + row * cell_h,
                w: cell_w,
                h: cell_h,
            },
            true,
        );
    }
}

/// Collect all windows eligible for [`distribute_clients`] together with the
/// monitor work area needed to lay them out.
///
/// Returns `(windows, work_rect)` where `work_rect` is the drawable area of
/// the monitor after subtracting the bar (i.e. `Monitor::work_rect`).  Using
/// `work_rect` directly means the bar offset is already baked in for both
/// top-bar and bottom-bar configurations, and no manual `y_offset` correction
/// is needed in the caller.
fn collect_distribute_targets(
    globals: &crate::globals::Globals,
    sel_mon_id: usize,
) -> (Vec<WindowId>, Rect) {
    let empty = (Vec::new(), Rect::default());

    let Some(mon) = globals.monitor(sel_mon_id) else {
        return empty;
    };

    let tag_set = mon.selected_tags();
    // work_rect already accounts for bar height and position (top or bottom),
    // so it is the correct region to fill with the grid.
    let work_rect = mon.work_rect;

    let mut wins = Vec::new();
    for (c_win, c) in mon.iter_clients(globals.clients.map()) {
        if c.isfloating
            && !c.isfixed
            && (c.tags & tag_set) != 0
            && c.snap_status == SnapPosition::None
        {
            wins.push(c_win);
        }
    }

    (wins, work_rect)
}
