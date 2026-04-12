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

use crate::animation::{MoveResizeMode, move_resize_client};
use crate::backend::BackendOps;
use crate::constants::animation::{BORDER_MULTIPLIER, DEFAULT_FRAME_COUNT};
use crate::contexts::WmCtx;
use crate::types::{Monitor, Rect};

pub fn monocle(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    // ── raise the selected client so it is visible while we animate ───────
    let is_animated = ctx.core_mut().globals_mut().behavior.animated
        && !ctx.core_mut().globals_mut().monitors.is_empty();

    if is_animated {
        let mon = ctx.core_mut().globals_mut().selected_monitor();
        if let Some(selected_window) = mon.sel {
            ctx.backend().raise_window(selected_window);
            ctx.backend().flush();
        }
    }

    // ── snapshot selected window before the loop ────────
    let selected_window = ctx.selected_client();
    let selected_tags = m.selected_tags();

    // ── resize every tiled client to fill the work area ───────────────────
    for &win in &m.clients {
        let Some(c) = ctx.core().client(win) else {
            continue;
        };

        // Skip non-tiled, hidden, or invisible clients
        if !c.is_tiled(selected_tags) {
            continue;
        }

        let border_width = c.border_width();

        // Only animate the currently selected window; snap everything else
        // immediately so there are no ghost windows flying around.
        let frames =
            if ctx.core_mut().globals_mut().behavior.animated && Some(win) == selected_window {
                DEFAULT_FRAME_COUNT
            } else {
                0
            };

        move_resize_client(
            ctx,
            win,
            &Rect {
                x: m.work_rect.x,
                y: m.work_rect.y,
                w: m.work_rect.w - BORDER_MULTIPLIER * border_width,
                h: m.work_rect.h - BORDER_MULTIPLIER * border_width,
            },
            MoveResizeMode::AnimateTo,
            frames,
        );
    }
}
