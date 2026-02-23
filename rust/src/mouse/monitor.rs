//! Monitor-switch helpers for interactive mouse operations.
//!
//! When the user drags or resizes a window across a monitor boundary these
//! functions detect the crossing and call [`send_mon`] + [`focus`] so the
//! window is correctly adopted by the new monitor.
//!
//! # Typical call flow
//!
//! ```text
//! move_mouse / resize_mouse loop ends
//!   └─► handle_client_monitor_switch(win)
//!             └─► reads client.geo
//!                   └─► handle_monitor_switch(win, &rect)
//!                             ├─► rect_to_mon_rect  → target monitor index
//!                             ├─► send_mon          → reassigns client
//!                             └─► focus(None)       → re-focus on new monitor
//! ```

use crate::client::unfocus_win;
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut};
use crate::monitor::rect_to_mon_rect;
use crate::monitor::send_mon;
use crate::types::*;
use x11rb::protocol::xproto::*;

/// Check whether `rect` lies on a different monitor than the currently
/// selected one and, if so, migrate the window and update `selmon`.
///
/// This is the low-level primitive.  Most call-sites should use
/// [`handle_client_monitor_switch`] which reads the rect from the client.
pub fn handle_monitor_switch(c_win: Window, rect: &Rect) {
    let new_mon = rect_to_mon_rect(rect);
    let current_mon = get_globals().selmon;

    let Some(target) = new_mon else { return };
    if target == current_mon {
        return;
    }

    // Unfocus the window on the old monitor before moving it.
    {
        let globals = get_globals();
        if let Some(cur_sel) = globals.monitors.get(current_mon).and_then(|m| m.sel) {
            unfocus_win(cur_sel, false);
        }
    }

    send_mon(c_win, target);

    {
        let globals = get_globals_mut();
        globals.selmon = target;
    }

    focus(None);
}

/// Convenience wrapper that reads the client's current geometry and delegates
/// to [`handle_monitor_switch`].
///
/// Call this at the end of every drag/resize loop so that windows dragged
/// across monitor boundaries are adopted by the correct monitor.
pub fn handle_client_monitor_switch(c_win: Window) {
    let rect = {
        let globals = get_globals();
        match globals.clients.get(&c_win) {
            Some(c) => c.geo,
            None => return,
        }
    };

    handle_monitor_switch(c_win, &rect);
}
