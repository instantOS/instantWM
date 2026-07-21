//! Batch operations across multiple floating windows.
//!
//! These functions operate on all floating clients of a monitor at once, as
//! opposed to the per-window helpers in the other sub-modules.
//!
//! # Functions
//!
//! | Function               | Purpose                                                  |
//! |------------------------|----------------------------------------------------------|
//! | `distribute_clients`   | Arrange all visible floating windows in an even grid     |

use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::types::*;

// ── Distribute ────────────────────────────────────────────────────────────────

/// Tile all visible, non-fixed, non-snapped floating windows on the selected
/// monitor into an evenly-spaced grid.
///
/// The grid dimensions are chosen so that the number of columns is the ceiling
/// of `sqrt(n)` (giving a roughly square layout), and rows are computed from
/// that.  Each cell receives one window, sized to exactly fill its cell.
///
/// Does nothing when there are no qualifying windows.
pub fn distribute_clients(ctx: &mut WmCtx) {
    let sel_mon_id = ctx.core().model().selected_monitor_id();

    let (floating_wins, work_rect) = collect_distribute_targets(ctx.core().model(), sel_mon_id);

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

        ctx.move_resize(
            win,
            Rect {
                x: work_rect.x + col * cell_w,
                y: work_rect.y + row * cell_h,
                w: cell_w,
                h: cell_h,
            },
            MoveResizeOptions::hinted_immediate(true),
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
    model: &crate::model::WmModel,
    sel_mon_id: MonitorId,
) -> (Vec<WindowId>, Rect) {
    let empty = (Vec::new(), Rect::default());

    let Some(mon) = model.monitor(sel_mon_id) else {
        return empty;
    };

    let tag_set = mon.selected_tags();
    // work_rect already accounts for bar height and position (top or bottom),
    // so it is the correct region to fill with the grid.
    let work_rect = mon.work_rect();

    let mut wins = Vec::new();
    for (c_win, c) in mon.iter_clients(&model.clients) {
        if c.mode.is_floating()
            && !c.is_fixed_size
            && c.tags.intersects(tag_set)
            && c.snap_status == SnapPosition::None
        {
            wins.push(c_win);
        }
    }

    (wins, work_rect)
}
