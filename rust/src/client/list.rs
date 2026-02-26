//! Linked-list management for the per-monitor client lists.
//!
//! Clients on each monitor are stored in two intrusive singly-linked lists:
//!
//! * **clients** – the "focus order" list, threaded through [`Client::next`].
//! * **stack**   – the "stacking order" list, threaded through [`Client::snext`].
//!
//! Both lists are headed by [`Monitor::clients`] / [`Monitor::stack`] respectively
//! and terminated by `None`.  All mutation goes through the helpers here so that
//! the invariants of both lists are maintained in one place.

use crate::contexts::WmCtx;
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut};
use crate::layouts::arrange;
use x11rb::protocol::xproto::Window;

// ---------------------------------------------------------------------------
// Client-list (focus order)
// ---------------------------------------------------------------------------

/// Prepend `win` to the front of its monitor's client list.
///
/// The monitor is determined by `Client::mon_id`.  Does nothing if the client
/// is not found in the global map or has no monitor assigned.
pub fn attach(win: Window) {
    let globals = get_globals_mut();
    let mon_id = globals.clients.get(&win).and_then(|c| c.mon_id);
    if let Some(mon_id) = mon_id {
        let old_head = globals.monitor(mon_id).and_then(|m| m.clients);
        if let Some(client) = globals.clients.get_mut(&win) {
            client.next = old_head;
        }
        if let Some(mon) = globals.monitor_mut(mon_id) {
            mon.clients = Some(win);
        }
    }
}

/// Remove `win` from its monitor's client list.
///
/// Walks the list to splice out `win`.  If `win` is not present the function
/// returns silently.
pub fn detach(win: Window) {
    let globals = get_globals_mut();
    let mon_id = match globals.clients.get(&win).and_then(|c| c.mon_id) {
        Some(id) => id,
        None => return,
    };

    let client_next = globals.clients.get(&win).and_then(|c| c.next);

    // Collect the traversal snapshot to avoid aliasing `clients` while mutating.
    let mut traversal: Vec<(Window, Option<Window>)> = Vec::new();
    let mut current = globals.monitor(mon_id).and_then(|m| m.clients);
    let mut prev: Option<Window> = None;

    while let Some(cur_win) = current {
        let next = globals.clients.get(&cur_win).and_then(|c| c.next);
        traversal.push((cur_win, prev));
        prev = Some(cur_win);
        current = next;
    }

    for (cur_win, prev_win) in traversal {
        if cur_win == win {
            match prev_win {
                Some(p) => {
                    if let Some(prev_client) = globals.clients.get_mut(&p) {
                        prev_client.next = client_next;
                    }
                }
                None => {
                    if let Some(mon) = globals.monitor_mut(mon_id) {
                        mon.clients = client_next;
                    }
                }
            }
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Stack list (stacking order)
// ---------------------------------------------------------------------------

/// Prepend `win` to the front of its monitor's stack list.
pub fn attach_stack(win: Window) {
    let globals = get_globals_mut();
    let mon_id = globals.clients.get(&win).and_then(|c| c.mon_id);
    if let Some(mon_id) = mon_id {
        let old_stack = globals.monitor(mon_id).and_then(|m| m.stack);
        if let Some(client) = globals.clients.get_mut(&win) {
            client.snext = old_stack;
        }
        if let Some(mon) = globals.monitor_mut(mon_id) {
            mon.stack = Some(win);
        }
    }
}

/// Remove `win` from its monitor's stack list.
///
/// Also updates `Monitor::sel` if `win` was the selected client: the first
/// visible, non-hidden client remaining in the stack becomes the new selection,
/// or `None` if no such client exists.
pub fn detach_stack(win: Window) {
    let globals = get_globals_mut();
    let mon_id = match globals.clients.get(&win).and_then(|c| c.mon_id) {
        Some(id) => id,
        None => return,
    };

    let client_snext = globals.clients.get(&win).and_then(|c| c.snext);

    // Snapshot the traversal to avoid aliasing.
    let mut traversal: Vec<(Window, Option<Window>)> = Vec::new();
    let mut current = globals.monitor(mon_id).and_then(|m| m.stack);
    let mut prev: Option<Window> = None;

    while let Some(cur_win) = current {
        let snext = globals.clients.get(&cur_win).and_then(|c| c.snext);
        traversal.push((cur_win, prev));
        prev = Some(cur_win);
        current = snext;
    }

    for (cur_win, prev_win) in traversal {
        if cur_win == win {
            match prev_win {
                Some(p) => {
                    if let Some(prev_client) = globals.clients.get_mut(&p) {
                        prev_client.snext = client_snext;
                    }
                }
                None => {
                    if let Some(mon) = globals.monitor_mut(mon_id) {
                        mon.stack = client_snext;
                    }
                }
            }

            // If `win` was selected, we don't update mon.sel here. 
            // We rely on focus() to discover the new selection from the stack.
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Traversal helpers
// ---------------------------------------------------------------------------

/// Return the first tiled (non-floating, visible, non-hidden) client starting
/// from `start_win` in the focus-order list.
pub fn next_tiled(start_win: Option<Window>) -> Option<Window> {
    let mut current = start_win;
    let globals = get_globals();

    while let Some(win) = current {
        if let Some(c) = globals.clients.get(&win) {
            let selected = c
                .mon_id
                .and_then(|mid| globals.monitor(mid))
                .map(|m| m.selected_tags())
                .unwrap_or(0);
            if !c.isfloating && c.is_visible_on_tags(selected) && !c.is_hidden {
                return Some(win);
            }
            current = c.next;
        } else {
            break;
        }
    }
    None
}

/// Detach `win` from the client list and re-attach it at the front (master
/// position), then re-focus and re-arrange the monitor.
pub fn pop(ctx: &mut WmCtx, win: Window) {
    detach(win);
    attach(win);
    let mon_id = ctx.g.clients.get(&win).and_then(|c| c.mon_id);
    focus(ctx, Some(win));

    if let Some(mid) = mon_id {
        arrange(ctx, Some(mid));
    }
}

// ---------------------------------------------------------------------------
// Lookup helpers
// ---------------------------------------------------------------------------

/// Returns `Some(win)` if `win` is a currently managed client, `None` otherwise.
///
/// This exists as a typed check: callers that only care *whether* a window is
/// managed can use this instead of reaching into `globals.clients` directly.
pub fn win_to_client(win: Window) -> Option<Window> {
    if get_globals().clients.contains_key(&win) {
        Some(win)
    } else {
        None
    }
}
