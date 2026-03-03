//! Monocle layout — every tiled client occupies the full work area.
//!
//! ```text
//! ┌─────────────────────────────┐
//! │                             │
//! │   client[0]  (on top)       │
//! │                             │
//! └─────────────────────────────┘
//! ```
//!
//! All tiled clients are resized to fill `work_rect` exactly.  Only the
//! selected client is raised to the top of the stack, so cycling through
//! clients feels like flipping through full-screen cards.
//!
//! The selected window is animated with the normal frame-count; every other
//! window is snapped into place instantly (0 frames) to avoid mid-air ghost
//! windows appearing during the animation.

use crate::animation::animate_client;
use crate::backend::BackendOps;
use crate::client::next_tiled_ctx;
use crate::constants::animation::{BORDER_MULTIPLIER, DEFAULT_FRAME_COUNT};
use crate::contexts::WmCtx;
use crate::types::{Monitor, Rect};

pub fn monocle(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    // ── raise the selected client so it is visible while we animate ───────
    let is_animated = ctx.g.animated && !ctx.g.monitors.is_empty();

    if is_animated {
        if let Some(mon) = ctx.g.selmon() {
            if let Some(sel_win) = mon.sel {
                ctx.backend.raise_window(sel_win);
                ctx.backend.flush();
            }
        }
    }

    // ── snapshot selected window before the loop ────────
    let sel_win = ctx.g.selected_win();

    // ── resize every tiled client to fill the work area ───────────────────
    let mut current_window = next_tiled_ctx(ctx, m.clients);
    while let Some(win) = current_window {
        let (border_width, next_client) = ctx
            .g
            .clients
            .get(&win)
            .map(|c| c.border_and_next())
            .unwrap_or((0, None));

        // Only animate the currently selected window; snap everything else
        // immediately so there are no ghost windows flying around.
        let frames = if ctx.g.animated && Some(win) == sel_win {
            DEFAULT_FRAME_COUNT
        } else {
            0
        };

        animate_client(
            ctx,
            win,
            &Rect {
                x: m.work_rect.x,
                y: m.work_rect.y,
                w: m.work_rect.w - BORDER_MULTIPLIER * border_width,
                h: m.work_rect.h - BORDER_MULTIPLIER * border_width,
            },
            frames,
            0,
        );

        current_window = next_tiled_ctx(ctx, next_client);
    }
}
