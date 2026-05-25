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

use crate::constants::animation::DEFAULT_FRAME_COUNT;
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::layouts::LayoutKind;
use crate::layouts::placement::LayoutPlacement;
use crate::types::Monitor;

pub fn monocle(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    // ── raise the selected client so it is visible while we animate ───────
    let is_animated = ctx.core_mut().globals_mut().behavior.animated
        && !ctx.core_mut().globals_mut().monitors.is_empty();

    if is_animated {
        let mon = ctx.core_mut().globals_mut().selected_monitor();
        if let Some(selected_window) = mon.sel {
            ctx.backend().raise_window_visual_only(selected_window);
            ctx.backend().flush();
        }
    }

    // ── snapshot selected window before the loop ────────
    let selected_window = ctx.core().globals().selected_win();
    let selected_tags = m.selected_tags();
    let tiled_client_count = m.tiled_client_count(ctx.core().globals().clients.map()) as u32;
    let placement = LayoutPlacement::new(
        ctx.core().globals(),
        m,
        LayoutKind::Monocle,
        tiled_client_count,
    );
    let work_rect = placement.work_rect();

    // ── resize every tiled client to fill the work area ───────────────────
    for &win in &m.clients {
        let Some(c) = ctx.core().globals().clients.get(&win) else {
            continue;
        };

        // Skip non-tiled, hidden, or invisible clients
        if !c.is_tiled(selected_tags) {
            continue;
        }

        let border_width = c.border_width;

        // Only animate the currently selected window; snap everything else
        // immediately so there are no ghost windows flying around.
        let frames =
            if ctx.core_mut().globals_mut().behavior.animated && Some(win) == selected_window {
                DEFAULT_FRAME_COUNT
            } else {
                0
            };

        placement.place(
            ctx,
            win,
            work_rect,
            border_width,
            MoveResizeOptions::animate_to(frames),
        );
    }
}
