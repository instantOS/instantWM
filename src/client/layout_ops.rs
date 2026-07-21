//! Layout-driven client operations.
//!
//! These are small operations that sit at the boundary between the client and
//! the layout engine.  They are collected here so that neither `geometry.rs`
//! nor the layout algorithms need to know about each other's internals.

use crate::contexts::WmCtx;
// ---------------------------------------------------------------------------
// zoom
// ---------------------------------------------------------------------------

/// Promote the selected window to the master position.
///
/// In manual tiling the "master" is the first visual leaf. [`zoom`] swaps the
/// selected window into that leaf without rebuilding the tree.
///
/// # Edge cases
///
/// * Does nothing when the current layout is not a tiling layout, or when the
///   selected client is floating.
/// * When the selected window **is already** the master, the *next* tiled
///   window is promoted instead (if one exists).  If there is no next tiled
///   window the function returns early.
pub fn zoom(ctx: &mut WmCtx) {
    if ctx.core().model().is_overview_active() {
        crate::overview::exit_overview(ctx, crate::overview::ExitMode::ToSelectedWindow);
        ctx.request_bar_update();
        return;
    }

    let Some(win) = ctx.core().model().selected_win() else {
        return;
    };

    // Raise the window immediately so it appears on top while the layout
    // catches up on the next arrange pass.
    ctx.window_backend().raise_window_visual_only(win);
    ctx.window_backend().flush();

    let Some(view) = ctx.core().model().client_view(win) else {
        return;
    };

    // Only meaningful in a tiling layout with a non-floating window.
    if !view.monitor.is_tiling_layout() || !view.client.mode.is_tiling() {
        return;
    }

    crate::layouts::promote_tree(ctx, win);
}
