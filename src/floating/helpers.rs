//! Small stateless helper functions used throughout the floating module.
//!
//! None of these functions mutate floating state – they only inspect it.

use crate::contexts::{WmCtx, WmCtxX11};
use crate::geometry::MoveResizeOptions;
use crate::model::WmModel;
use crate::types::WindowId;

// ── Layout query ─────────────────────────────────────────────────────────────

/// Returns `true` if the currently selected monitor has a tiling layout active.
///
/// Used as a guard throughout the floating module: floating-only operations
/// should be no-ops when a tiling layout is active and the window is not
/// explicitly floating.
pub fn has_tiling_layout(model: &WmModel) -> bool {
    model.expect_selected_monitor().is_tiling_layout()
}

// ── Geometry helpers ──────────────────────────────────────────────────────────

/// Nudge the client one pixel to the right and back, forcing a layout refresh.
///
/// This is a lightweight way to make the X server re-evaluate size hints and
/// repaint the window frame without triggering a full `arrange()` pass.  It is
/// used after restoring a saved geometry so the window manager picks up the
/// correct position.
pub fn apply_size(ctx: &mut WmCtxX11<'_>, win: WindowId) {
    let geo = ctx.core.model().client(win).map(|c| c.geo);
    if let Some(mut rect) = geo {
        rect.x += 1;
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        wm_ctx.move_resize(win, rect, MoveResizeOptions::immediate());
    }
}
