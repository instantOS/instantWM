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

use std::collections::HashMap;

use crate::constants::animation::BORDER_MULTIPLIER;
use crate::geometry::MoveResizeOptions;
use crate::layouts::LayoutOutput;
use crate::types::client::Client;
use crate::types::{Monitor, Rect, WindowId};

// ── public entry points ───────────────────────────────────────────────────────

/// Inward-spiral fibonacci layout.
pub fn spiral(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
    _animated: bool,
) -> Vec<LayoutOutput> {
    fibonacci(monitor, clients, _animated, true)
}

/// Outward-dwindle fibonacci layout.
pub fn dwindle(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
    _animated: bool,
) -> Vec<LayoutOutput> {
    fibonacci(monitor, clients, _animated, false)
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
///   inward (classic golden-ratio spiral).
/// - `spiral = false` — the client takes the inner half; the remainder grows
///   outward (dwindle / Fibonacci staircase).
fn fibonacci(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
    _animated: bool,
    spiral: bool,
) -> Vec<LayoutOutput> {
    // ── count tiled clients ───────────────────────────────────────────────
    let n = monitor.tiled_client_count(clients) as u32;

    if n == 0 {
        return Vec::new();
    }

    // ── iteratively partition the work area ───────────────────────────────
    // `x`, `y`, `w`, `h` track the *remaining* rectangle that still needs to
    // be distributed among the not-yet-placed clients.
    let mut x = monitor.work_rect.x;
    let mut y = monitor.work_rect.y;
    let mut w = monitor.work_rect.w;
    let mut h = monitor.work_rect.h;

    let selected_tags = monitor.selected_tags();
    let mut i: u32 = 0;

    let mut result = Vec::new();

    for &win in &monitor.clients {
        let Some(c) = clients.get(&win) else {
            continue;
        };

        // Skip non-tiled, hidden, or invisible clients
        if !c.is_tiled(selected_tags) {
            continue;
        }

        let border_width = c.border_width;

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

        result.push(LayoutOutput {
            win,
            rect: Rect {
                x,
                y,
                w: w - BORDER_MULTIPLIER * border_width,
                h: h - BORDER_MULTIPLIER * border_width,
            },
            options: MoveResizeOptions::hinted_immediate(false),
        });

        // After placing the client, advance the remainder pointer.
        if i.is_multiple_of(2) {
            if !spiral {
                y += h;
            }
        } else if spiral {
            x += w;
        }

        i += 1;
    }

    result
}
