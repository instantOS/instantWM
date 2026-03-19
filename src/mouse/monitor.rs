//! Monitor-switch helpers for interactive mouse operations.
//!
//! When the user drags or resizes a window across a monitor boundary these
//! functions detect the crossing and call [`transfer_client`] + [`focus`] so the
//! window is correctly adopted by the new monitor.
//!
//! # Typical call flow
//!
//! ```text
//! move_mouse / resize_mouse loop ends
//!   └─► handle_client_monitor_switch(win)
//!             └─► reads client.geo
//!                   └─► handle_monitor_switch(win, &rect)
//!                             ├─► find_monitor_by_rect → target monitor index
//!                             ├─► transfer_client   → reassigns client
//!                             └─► focus(None)       → re-focus on new monitor
//! ```

use crate::contexts::WmCtx;
use crate::focus::unfocus_win;
use crate::monitor::transfer_client;
use crate::types::*;

/// Check whether `rect` lies on a different monitor than the currently
/// selected one and, if so, migrate the window and update `selmon`.
///
/// This is the low-level primitive.  Most call-sites should use
/// [`handle_client_monitor_switch`] which reads the rect from the client.
///
/// # Parameters
///
/// * `ctx` - The mouse context containing monitor state
/// * `c_win` - The client window to potentially move
/// * `rect` - The window's geometry to check against monitor boundaries
pub fn handle_monitor_switch(ctx: &mut WmCtx, c_win: WindowId, rect: &Rect) {
    if ctx.is_wayland() {
        return;
    }
    let new_mon =
        crate::types::find_monitor_by_rect(ctx.core_mut().globals_mut().monitors.monitors(), rect)
            .or(Some(ctx.core_mut().globals_mut().selected_monitor_id()));

    let current_mon = ctx.core_mut().globals_mut().selected_monitor_id();

    let Some(target) = new_mon else { return };
    if target == current_mon {
        return;
    }

    // Unfocus the window on the old monitor before moving it.
    if let Some(cur_sel) = ctx
        .core_mut()
        .globals_mut()
        .monitor(current_mon)
        .and_then(|m| m.sel)
    {
        unfocus_win(ctx, cur_sel, false);
    }

    transfer_client(ctx, c_win, target);

    ctx.core_mut().globals_mut().set_selected_monitor(target);
    crate::focus::focus_soft(ctx, None);
}

/// Convenience wrapper that reads the client's current geometry and delegates
/// to [`handle_monitor_switch`].
///
/// Call this at the end of every drag/resize loop so that windows dragged
/// across monitor boundaries are adopted by the correct monitor.
///
/// # Parameters
///
/// * `ctx` - The mouse context containing client and monitor state
/// * `c_win` - The client window to check and potentially move
pub fn handle_client_monitor_switch(ctx: &mut WmCtx, c_win: WindowId) {
    if ctx.is_wayland() {
        return;
    }
    let Some(c) = ctx.core_mut().globals_mut().clients.get(&c_win) else {
        return;
    };
    let rect = c.geo;

    handle_monitor_switch(ctx, c_win, &rect);
}
