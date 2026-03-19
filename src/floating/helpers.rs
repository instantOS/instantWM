//! Small stateless helper functions used throughout the floating module.
//!
//! None of these functions mutate floating state – they only inspect it.

use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
use crate::types::WindowId;

// ── Layout query ─────────────────────────────────────────────────────────────

/// Returns `true` if the currently selected monitor has a tiling layout active.
///
/// Used as a guard throughout the floating module: floating-only operations
/// should be no-ops when a tiling layout is active and the window is not
/// explicitly floating.
pub fn has_tiling_layout(core: &CoreCtx) -> bool {
    core.globals().selected_monitor().is_tiling_layout()
}

// ── Per-client queries ────────────────────────────────────────────────────────

/// Returns `true` if the client should be treated as floating right now.
///
/// A client is considered floating when either:
/// - its `isfloating` flag is set, or
/// - no tiling layout is active on the selected monitor (all windows float in
///   floating-only layouts).
pub fn check_floating(core: &CoreCtx, win: WindowId) -> bool {
    if let Some(client) = core.globals().clients.get(&win) {
        if client.is_floating {
            return true;
        }
        if !core.globals().selected_monitor().is_tiling_layout() {
            return true;
        }
    }
    false
}

// ── Geometry helpers ──────────────────────────────────────────────────────────

/// Nudge the client one pixel to the right and back, forcing a layout refresh.
///
/// This is a lightweight way to make the X server re-evaluate size hints and
/// repaint the window frame without triggering a full `arrange()` pass.  It is
/// used after restoring a saved geometry so the window manager picks up the
/// correct position.
pub fn apply_size(ctx: &mut WmCtxX11<'_>, win: WindowId) {
    let geo = ctx.core.globals().clients.get(&win).map(|c| c.geo);
    if let Some(mut rect) = geo {
        rect.x += 1;
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        wm_ctx.resize_client(win, rect);
    }
}
