//! Layout-driven client operations.
//!
//! These are small operations that sit at the boundary between the client and
//! the layout engine.  They are collected here so that neither `geometry.rs`
//! nor the layout algorithms need to know about each other's internals.

use crate::backend::BackendOps;
use crate::contexts::WmCtx;
use crate::focus::focus_soft;

// ---------------------------------------------------------------------------
// zoom
// ---------------------------------------------------------------------------

fn pop(ctx: &mut WmCtx, win: crate::types::WindowId) {
    ctx.core_mut().globals_mut().detach(win);
    ctx.core_mut().globals_mut().attach(win);
    let monitor_id = ctx.core().globals().clients.monitor_id(win);
    focus_soft(ctx, Some(win));

    if let Some(mid) = monitor_id {
        ctx.core_mut()
            .globals_mut()
            .queue_layout_for_monitor_urgent(mid);
    }
}

// ---------------------------------------------------------------------------
// zoom
// ---------------------------------------------------------------------------

/// Promote the selected window to the master position.
///
/// In a tiling layout the "master" is the first client in the focus-order
/// list (the leftmost / largest slot, depending on the layout algorithm).
/// [`zoom`] moves the selected window there via [`pop`].
///
/// # Edge cases
///
/// * Does nothing when the current layout is not a tiling layout, or when the
///   selected client is floating.
/// * When the selected window **is already** the master, the *next* tiled
///   window is promoted instead (if one exists).  If there is no next tiled
///   window the function returns early.
pub fn zoom(ctx: &mut WmCtx) {
    let Some(win) = ctx.selected_client() else {
        return;
    };

    // Raise the window immediately so it appears on top while the layout
    // catches up on the next arrange pass.
    ctx.backend().raise_window_visual_only(win);
    ctx.backend().flush();

    let (is_tiling_mode, monitor_id) = ctx
        .core()
        .globals()
        .clients
        .get(&win)
        .map(|c| (c.mode.is_tiling(), c.monitor_id))
        .unwrap_or((false, crate::types::MonitorId(0)));

    let Some(mon) = ctx.core().globals().monitor(monitor_id) else {
        return;
    };

    // Only meaningful in a tiling layout with a non-floating window.
    if !mon.is_tiling_layout() || !is_tiling_mode {
        return;
    }

    // Find the current master (first tiled client on the monitor).
    let first_on_monitor = mon.clients.first().copied();
    let first_tiled =
        first_on_monitor.and_then(|w| mon.next_tiled(ctx.core().globals().clients.map(), Some(w)));

    if first_tiled == Some(win) {
        let next = mon.next_tiled(ctx.core().globals().clients.map(), first_tiled);

        // Nothing to promote if there is only one tiled window.
        if next.is_none() {
            return;
        }
    }

    pop(ctx, win);
}
