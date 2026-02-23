#![allow(dead_code)]
//! Fibonacci / golden-ratio layout algorithms.
//!
//! Two variants share a single recursive splitting implementation:
//!
//! ## `spiral` — inward spiral
//!
//! ```text
//! ┌───────────────┬───────────┐
//! │               │     1     │
//! │       0       ├─────┬─────┤
//! │               │  3  │    │
//! ├───────┬───────┤     │  2 │
//! │       │       ├──┬──┘    │
//! │   5   │   4   │6 │       │
//! └───────┴───────┴──┴───────┘
//! ```
//!
//! Each new client is placed by halving the *current* remaining rect and
//! spiraling inward — the split axis alternates every two clients and the
//! "remainder" rect always advances toward the centre.
//!
//! ## `dwindle` — outward dwindle
//!
//! ```text
//! ┌───────────────┬───────────┐
//! │               │     1     │
//! │       0       ├─────┬─────┤
//! │               │  2  │    │
//! ├───────────────┤     │  3 │
//! │               ├──┬──┘    │
//! │       4       │5 │   6   │
//! └───────────────┴──┴───────┘
//! ```
//!
//! Same halving logic as `spiral` but the "remainder" rect advances away from
//! the centre, producing a dwindling-strip pattern instead.
//!
//! ---
//!
//! The key difference is controlled by the `spiral` flag passed to
//! [`fibonacci`]:
//!
//! | flag    | how the remainder rect moves |
//! |---------|------------------------------|
//! | `true`  | toward the centre (spiral)   |
//! | `false` | away from centre (dwindle)   |

use crate::client::{next_tiled, resize};
use crate::globals::get_globals;
use crate::types::{Monitor, Rect};

// ── public entry points ───────────────────────────────────────────────────────

/// Inward-spiral fibonacci layout.
#[inline]
pub fn spiral(m: &mut Monitor) {
    fibonacci(m, true);
}

/// Outward-dwindle fibonacci layout.
#[inline]
pub fn dwindle(m: &mut Monitor) {
    fibonacci(m, false);
}

// ── shared implementation ─────────────────────────────────────────────────────

/// Core fibonacci tiling algorithm.
///
/// Clients are placed by repeatedly halving the remaining work area.  The
/// split axis alternates every two clients (`i % 2`):
///
/// - Even `i` → split horizontally (top / bottom half)
/// - Odd  `i` → split vertically   (left / right half)
///
/// The `spiral` flag controls which half becomes the *current client's rect*
/// and which becomes the *remainder* for subsequent clients:
///
/// - `spiral = true`  — the client takes the outer half; the remainder shrinks
///                      inward (classic golden-ratio spiral).
/// - `spiral = false` — the client takes the inner half; the remainder grows
///                      outward (dwindle / Fibonacci staircase).
pub fn fibonacci(m: &mut Monitor, spiral: bool) {
    // ── count tiled clients ───────────────────────────────────────────────
    let mut n: u32 = 0;
    let mut c_win = next_tiled(m.clients);
    while c_win.is_some() {
        n += 1;
        let g = get_globals();
        c_win = c_win.and_then(|w| g.clients.get(&w)?.next);
    }

    if n == 0 {
        return;
    }

    // ── iteratively partition the work area ───────────────────────────────
    // `x`, `y`, `w`, `h` track the *remaining* rectangle that still needs to
    // be distributed among the not-yet-placed clients.
    let mut x = m.work_rect.x;
    let mut y = m.work_rect.y;
    let mut w = m.work_rect.w;
    let mut h = m.work_rect.h;

    let mut i: u32 = 0;
    let mut c_win = next_tiled(m.clients);

    while let Some(win) = c_win {
        let (border_width, next_client) = {
            let g = get_globals();
            g.clients
                .get(&win)
                .map(|c| (c.border_width, c.next))
                .unwrap_or((0, None))
        };

        // Split the remaining rect starting from the second client.
        if i > 0 {
            if i.is_multiple_of(2) {
                // Horizontal split — top half goes to the current client.
                h /= 2;
                // In spiral mode the remainder moves downward (toward centre);
                // in dwindle mode the client moves downward instead.
                if spiral {
                    y += h;
                }
            } else {
                // Vertical split — left half goes to the current client.
                w /= 2;
                // In dwindle mode the client moves to the right half;
                // in spiral mode the remainder moves right.
                if !spiral {
                    x += w;
                }
            }
        }

        resize(
            win,
            &Rect {
                x,
                y,
                w: w - 2 * border_width,
                h: h - 2 * border_width,
            },
            false,
        );

        // After placing the client, advance the remainder pointer.
        if i.is_multiple_of(2) {
            if !spiral {
                y += h;
            }
        } else if spiral {
            x += w;
        }

        i += 1;
        c_win = next_tiled(next_client);
    }
}
