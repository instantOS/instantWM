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
use crate::globals::get_globals;
use crate::types::*;
use x11rb::protocol::xproto::Window;

// ── Save / restore all floating ───────────────────────────────────────────────

/// Snapshot the geometry of every non-snapped floating client on `mon_id`.
///
/// Only clients whose tag belongs to a tag that currently has **no** tiling
/// layout (i.e. a pure floating tag) are included.  Snapped windows are
/// excluded because their geometry is managed by the snap system, not by free
/// floating.
///
/// Pair with [`restore_all_floating`] to round-trip positions across a layout
/// change (e.g. entering / leaving overview mode).
pub fn save_all_floating(mon_id: Option<usize>) {
    let Some(mid) = mon_id else { return };

    let wins_to_save = collect_floating_wins(mid);
    for win in wins_to_save {
        super::state::save_floating_win(win);
    }
}

/// Restore the geometry of every non-snapped floating client on `mon_id`.
///
/// Counterpart to [`save_all_floating`]: resizes each window back to the rect
/// that was captured by the most recent `save_all_floating` call.
pub fn restore_all_floating(mon_id: Option<usize>) {
    let Some(mid) = mon_id else { return };

    let wins_to_restore = collect_floating_wins(mid);
    for win in wins_to_restore {
        super::state::restore_floating_win(win);
    }
}

/// Walk `mon_id`'s client list and return all windows that are:
/// - on a tag that has no tiling layout active, and
/// - not currently snapped.
///
/// This is the shared selection logic for both save and restore.
fn collect_floating_wins(mid: usize) -> Vec<Window> {
    let globals = get_globals();

    let Some(mon) = globals.monitors.get(mid) else {
        return Vec::new();
    };

    let numtags = globals.tags.count();
    let mut wins = Vec::new();

    for tag_idx in 0..numtags {
        // Skip tags that have a tiling layout — only purely-floating tags matter.
        let tag_is_floating = match globals.tags.tags.get(tag_idx) {
            Some(tag) => !tag.layouts.is_tiling(),
            _ => false,
        };

        if !tag_is_floating {
            continue;
        }

        let tag_mask = 1u32 << tag_idx;
        let mut current = mon.clients;

        while let Some(c_win) = current {
            match globals.clients.get(&c_win) {
                Some(c) => {
                    if (c.tags & tag_mask) != 0 && c.snapstatus == SnapPosition::None {
                        wins.push(c_win);
                    }
                    current = c.next;
                }
                None => break,
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
pub fn distribute_clients() {
    let globals = get_globals();
    let sel_mon_id = globals.selmon;

    let (floating_wins, work_rect) = collect_distribute_targets(sel_mon_id);

    if floating_wins.is_empty() {
        return;
    }

    let n = floating_wins.len();

    // Choose a roughly-square grid.
    let cols = (n as f32).sqrt().ceil() as i32;
    let rows = ((n as f32) / (cols as f32)).ceil() as i32;

    let cell_w = work_rect.w / cols;
    let cell_h = work_rect.h / rows;

    for (i, win) in floating_wins.into_iter().enumerate() {
        let col = (i as i32) % cols;
        let row = (i as i32) / cols;

        resize(
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
fn collect_distribute_targets(sel_mon_id: usize) -> (Vec<Window>, Rect) {
    let globals = get_globals();

    let empty = (Vec::new(), Rect::default());

    let Some(mon) = globals.monitors.get(sel_mon_id) else {
        return empty;
    };

    let tagset = mon.tagset[mon.seltags as usize];
    // work_rect already accounts for bar height and position (top or bottom),
    // so it is the correct region to fill with the grid.
    let work_rect = mon.work_rect;

    let mut wins = Vec::new();
    let mut current = mon.clients;

    while let Some(c_win) = current {
        match globals.clients.get(&c_win) {
            Some(c) => {
                if c.isfloating
                    && !c.isfixed
                    && (c.tags & tagset) != 0
                    && c.snapstatus == SnapPosition::None
                {
                    wins.push(c_win);
                }
                current = c.next;
            }
            None => break,
        }
    }

    (wins, work_rect)
}
