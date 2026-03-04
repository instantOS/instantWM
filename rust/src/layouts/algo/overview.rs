//! Overview layout — a bird's-eye view of every client across all tags.
//!
//! All clients (regardless of their tag) are arranged in a square grid so the
//! user can see everything at a glance before jumping to a tag.
//!
//! ```text
//! ┌──────────┬──────────┬──────────┐
//! │ client 0 │ client 1 │ client 2 │
//! ├──────────┼──────────┼──────────┤
//! │ client 3 │ client 4 │ client 5 │
//! ├──────────┼──────────┼──────────┤
//! │ client 6 │ client 7 │ client 8 │
//! └──────────┴──────────┴──────────┘
//! ```
//!
//! ## Key behaviours
//!
//! - The grid size is the smallest integer `g` such that `g² ≥ total_clients`.
//! - Each cell is `work_width / g` wide and `work_height / g` tall.
//! - Clients retain their own width/height inside the cell (they are not
//!   stretched); only their x/y origin is repositioned.
//! - Floating clients have their float geometry saved before being moved.
//! - Overlay and hidden (`oldstate != 0`) clients are skipped entirely.
//! - After placement every client is raised above the bar window so the
//!   overview is fully visible even on monitors with topbar enabled.

use crate::backend::BackendOps;
use crate::client::resize;
use crate::contexts::WmCtx;
use crate::layouts::query::all_client_count;
use crate::types::{Monitor, Rect, WindowId};

/// Save the current geometry as the client's floating geometry.
///
/// Called before repositioning a floating client so that its position can be
/// restored when leaving overview mode.
fn save_floating(ctx: &mut WmCtx<'_>, win: WindowId) {
    if let Some(c) = ctx.g.clients.get_mut(&win) {
        c.float_geo = c.geo;
    }
}

pub fn overviewlayout(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    let n = all_client_count(ctx.g);
    if n == 0 {
        return;
    }

    // ── grid dimensions ───────────────────────────────────────────────────
    // Find the smallest g such that g² ≥ n (i.e. ceil(sqrt(n))).
    let mut gridwidth: i32 = 1;
    while gridwidth * gridwidth < n {
        gridwidth += 1;
    }

    // ── snapshot monitor geometry ─────────────────────────────────────────
    if ctx.g.monitors.is_empty() {
        return;
    }
    let (mon_x, mon_y, work_h, work_w, showbar) = match ctx.g.selected_monitor() {
        Some(mon) => (
            mon.monitor_rect.x,
            mon.monitor_rect.y,
            mon.work_rect.h,
            mon.work_rect.w,
            mon.showbar,
        ),
        None => return,
    };

    let bar_height = ctx.g.cfg.bar_height;

    // ── cell dimensions ───────────────────────────────────────────────────
    let cell_w = work_w / gridwidth;
    let cell_h = work_h / gridwidth;

    // Origin of the first cell — respect the bar if it is shown.
    let origin_x = mon_x;
    let origin_y = mon_y + if showbar { bar_height } else { 0 };

    let mut cur_x = origin_x;
    let mut cur_y = origin_y;

    // ── place every client ────────────────────────────────────────────────
    for &win in &m.clients {
        let c = match ctx.g.clients.get(&win) {
            Some(c) => c,
            None => continue,
        };

        let is_hidden = c.oldstate != 0;
        let is_overlay = m.overlay == Some(win);

        if is_hidden || is_overlay {
            continue;
        }

        // Keep the client's own dimensions; only reposition it.
        let client_w = c.geo.w;
        let client_h = c.geo.h;
        let is_floating = c.isfloating;

        // Persist float geometry so restore works after leaving overview.
        if is_floating {
            save_floating(ctx, win);
        }

        resize(
            ctx,
            win,
            &Rect {
                x: cur_x,
                y: cur_y,
                w: client_w,
                h: client_h,
            },
            false,
        );

        // Raise each client above the bar so nothing is obscured.
        ctx.backend.raise_window(win);

        // Advance to the next cell, wrapping to the next row.
        if cur_x + cell_w < mon_x + work_w {
            cur_x += cell_w;
        } else {
            cur_x = origin_x;
            cur_y += cell_h;
        }
    }

    ctx.backend.flush();
}
