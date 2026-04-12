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
use crate::floating::save_floating_geometry;
use crate::types::{Monitor, Rect};

pub fn overviewlayout(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    let n = ctx.core_mut().globals_mut().clients.len();
    if n == 0 {
        return;
    }

    // ── grid dimensions ───────────────────────────────────────────────────
    // Find the smallest g such that g² ≥ n (i.e. ceil(sqrt(n))).
    let mut gridwidth: i32 = 1;
    while ((gridwidth * gridwidth) as usize) < n {
        gridwidth += 1;
    }

    // ── snapshot monitor geometry ─────────────────────────────────────────
    if ctx.core_mut().globals_mut().monitors.is_empty() {
        return;
    }
    let showbar = m.showbar_for_mask(m.selected_tags());
    let (mon_x, mon_y, work_h, work_w) = (
        m.monitor_rect.x,
        m.monitor_rect.y,
        m.work_rect.h,
        m.work_rect.w,
    );

    let bar_height = ctx.core_mut().globals_mut().cfg.bar_height;

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
        let c = match ctx.core().client(win) {
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
        let is_floating = c.is_floating;

        // Persist float geometry so restore works after leaving overview.
        if is_floating && let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
            save_floating_geometry(client);
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
        ctx.backend().raise_window(win);

        // Advance to the next cell, wrapping to the next row.
        if cur_x + cell_w < mon_x + work_w {
            cur_x += cell_w;
        } else {
            cur_x = origin_x;
            cur_y += cell_h;
        }
    }

    ctx.backend().flush();
}
