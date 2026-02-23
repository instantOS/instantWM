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

use crate::animation::animate_client_rect;
use crate::client::next_tiled;
use crate::globals::{get_globals, get_x11};
use crate::types::{Monitor, Rect};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

pub fn monocle(m: &mut Monitor) {
    // ── raise the selected client so it is visible while we animate ───────
    {
        let g = get_globals();
        let is_animated = g.animated && !g.monitors.is_empty();

        if is_animated {
            if let Some(mon) = g.monitors.get(g.selmon) {
                if let Some(sel_win) = mon.sel {
                    let x11 = get_x11();
                    if let Some(ref conn) = x11.conn {
                        let _ = configure_window(
                            conn,
                            sel_win,
                            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                        );
                        let _ = conn.flush();
                    }
                }
            }
        }
    }

    // ── snapshot animation state & selected window before the loop ────────
    let (animated, sel_win) = {
        let g = get_globals();
        let sel = g.monitors.get(g.selmon).and_then(|mon| mon.sel);
        (g.animated, sel)
    };

    // ── resize every tiled client to fill the work area ───────────────────
    let mut c_win = next_tiled(m.clients);
    while let Some(win) = c_win {
        let (border_width, next_client) = {
            let g = get_globals();
            g.clients
                .get(&win)
                .map(|c| (c.border_width, c.next))
                .unwrap_or((0, None))
        };

        // Only animate the currently selected window; snap everything else
        // immediately so there are no ghost windows flying around.
        let frames = if animated && Some(win) == sel_win {
            7
        } else {
            0
        };

        animate_client_rect(
            win,
            &Rect {
                x: m.work_rect.x,
                y: m.work_rect.y,
                w: m.work_rect.w - 2 * border_width,
                h: m.work_rect.h - 2 * border_width,
            },
            frames,
            0,
        );

        c_win = next_tiled(next_client);
    }
}
