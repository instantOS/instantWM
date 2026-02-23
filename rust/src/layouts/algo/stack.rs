#![allow(dead_code)]
//! Stacking layout algorithms: deck, bstack, and bstackhoriz.
//!
//! All three share a master-area / stack-area split, but differ in orientation
//! and how the stack area is arranged:
//!
//! ## `deck` — vertical master, single stacked card
//!
//! ```text
//! ┌──────────────┬──────────────┐
//! │  master[0]   │              │
//! ├──────────────┤  stack card  │
//! │  master[1]   │  (all stack  │
//! ├──────────────┤   clients    │
//! │  master[2]   │   overlap)   │
//! └──────────────┴──────────────┘
//! ```
//!
//! Stack clients are all resized to the same rect — only the top one is
//! visible. Useful for tabbed-style workflows where you cycle through stack
//! clients one at a time.
//!
//! ## `bstack` — horizontal master row, vertical stack columns
//!
//! ```text
//! ┌──────────────────────────────┐
//! │  master[0]  │  master[1]    │
//! ├─────────┬───┴──┬────────────┤
//! │ stack[0]│stack[1]│ stack[2] │
//! └─────────┴────────┴──────────┘
//! ```
//!
//! ## `bstackhoriz` — horizontal master row, horizontal stack rows
//!
//! ```text
//! ┌──────────────────────────────┐
//! │  master[0]  │  master[1]    │
//! ├──────────────────────────────┤
//! │           stack[0]          │
//! ├──────────────────────────────┤
//! │           stack[1]          │
//! └──────────────────────────────┘
//! ```

use crate::animation::animate_client_rect;
use crate::client::{client_height, client_width, next_tiled, resize};
use crate::globals::get_globals;
use crate::layouts::query::client_count;
use crate::types::{Monitor, Rect};
use crate::util::min;

// ── deck ─────────────────────────────────────────────────────────────────────

/// Deck layout.
///
/// The master column is split vertically among the first `nmaster` clients.
/// All stack clients are placed on top of each other in the remaining area —
/// only the topmost is visible, giving a card-deck feel.
pub fn deck(m: &mut Monitor) {
    // ── count tiled clients ───────────────────────────────────────────────
    let mut n: u32 = 0;
    let mut c_win = next_tiled(m.clients);
    while c_win.is_some() {
        n += 1;
        let g = get_globals();
        c_win = c_win.and_then(|w| g.clients.get(&w)?.next.into());
    }

    if n == 0 {
        return;
    }

    // ── master-column width ───────────────────────────────────────────────
    let mw: u32 = if n > m.nmaster as u32 {
        if m.nmaster > 0 {
            (m.mfact * m.work_rect.w as f32) as u32
        } else {
            0
        }
    } else {
        m.work_rect.w as u32
    };

    // ── place each client ─────────────────────────────────────────────────
    let mut my: u32 = 0; // running y-offset inside master column
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

        if i < m.nmaster as u32 {
            // ── master client — animated vertical split ───────────────────
            let h = (m.work_rect.h - my as i32) / (min(n, m.nmaster as u32) - i) as i32;
            resize(
                win,
                &Rect {
                    x: m.work_rect.x,
                    y: m.work_rect.y + my as i32,
                    w: mw as i32 - 2 * border_width,
                    h: h - 2 * border_width,
                },
                false,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                my += client_height(c) as u32;
            }
        } else {
            // ── stack client — all overlap in the same rect ───────────────
            resize(
                win,
                &Rect {
                    x: m.work_rect.x + mw as i32,
                    y: m.work_rect.y,
                    w: m.work_rect.w - mw as i32 - 2 * border_width,
                    h: m.work_rect.h - 2 * border_width,
                },
                false,
            );
        }

        i += 1;
        c_win = next_tiled(next_client);
    }
}

// ── bstack ────────────────────────────────────────────────────────────────────

/// Bottom-stack layout.
///
/// The first `nmaster` clients share a horizontal master row at the top.
/// Remaining clients are divided into equal-width vertical columns below.
pub fn bstack(m: &mut Monitor) {
    let framecount = {
        let g = get_globals();
        if g.animated && client_count() > 4 {
            4
        } else {
            7
        }
    };

    // ── count tiled clients ───────────────────────────────────────────────
    let mut n: u32 = 0;
    let mut c_win = next_tiled(m.clients);
    while c_win.is_some() {
        n += 1;
        let g = get_globals();
        c_win = c_win.and_then(|w| g.clients.get(&w)?.next.into());
    }

    if n == 0 {
        return;
    }

    // ── geometry ──────────────────────────────────────────────────────────
    // mh  — master row height
    // tw  — width of each stack column
    // ty  — top-y of the stack row
    let (mh, tw, ty) = if n > m.nmaster as u32 {
        let mh = if m.nmaster > 0 {
            (m.mfact * m.work_rect.h as f32) as i32
        } else {
            0
        };
        let tw = m.work_rect.w / (n - m.nmaster as u32) as i32;
        let ty = m.work_rect.y + mh;
        (mh, tw, ty)
    } else {
        (m.work_rect.h, m.work_rect.w, m.work_rect.y)
    };

    // ── place each client ─────────────────────────────────────────────────
    let mut mx: i32 = 0; // running x-offset inside master row
    let mut tx: i32 = m.work_rect.x; // running x-offset inside stack row
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

        if i < m.nmaster as u32 {
            // ── master client — horizontal slice of the top row ───────────
            let w = (m.work_rect.w - mx) / (min(n, m.nmaster as u32) - i) as i32;
            animate_client_rect(
                win,
                &Rect {
                    x: m.work_rect.x + mx,
                    y: m.work_rect.y,
                    w: w - 2 * border_width,
                    h: mh - 2 * border_width,
                },
                framecount,
                0,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                mx += client_width(c);
            }
        } else {
            // ── stack client — column in the bottom row ───────────────────
            let h = m.work_rect.h - mh;
            animate_client_rect(
                win,
                &Rect {
                    x: tx,
                    y: ty,
                    w: tw - 2 * border_width,
                    h: h - 2 * border_width,
                },
                framecount,
                0,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                if tw != m.work_rect.w {
                    tx += client_width(c);
                }
            }
        }

        i += 1;
        c_win = next_tiled(next_client);
    }
}

// ── bstackhoriz ───────────────────────────────────────────────────────────────

/// Horizontal bottom-stack layout.
///
/// Like [`bstack`] but stack clients are arranged as horizontal rows rather
/// than vertical columns — each stack client spans the full work width.
pub fn bstackhoriz(m: &mut Monitor) {
    let framecount = {
        let g = get_globals();
        if g.animated && client_count() > 4 {
            4
        } else {
            7
        }
    };

    // ── count tiled clients ───────────────────────────────────────────────
    let mut n: u32 = 0;
    let mut c_win = next_tiled(m.clients);
    while c_win.is_some() {
        n += 1;
        let g = get_globals();
        c_win = c_win.and_then(|w| g.clients.get(&w)?.next.into());
    }

    if n == 0 {
        return;
    }

    // ── geometry ──────────────────────────────────────────────────────────
    // mh  — master row height
    // th  — height of each stack row
    // ty  — top-y of the first stack row (mutable, advances per client)
    let (mh, th, mut ty) = if n > m.nmaster as u32 {
        let mh = if m.nmaster > 0 {
            (m.mfact * m.work_rect.h as f32) as i32
        } else {
            0
        };
        let th = (m.work_rect.h - mh) / (n - m.nmaster as u32) as i32;
        let ty = m.work_rect.y + mh;
        (mh, th, ty)
    } else {
        (m.work_rect.h, m.work_rect.h, m.work_rect.y)
    };

    // ── place each client ─────────────────────────────────────────────────
    let mut mx: i32 = 0; // running x-offset inside master row
    let tx: i32 = m.work_rect.x;
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

        if i < m.nmaster as u32 {
            // ── master client — horizontal slice of the top row ───────────
            let w = (m.work_rect.w - mx) / (min(n, m.nmaster as u32) - i) as i32;
            animate_client_rect(
                win,
                &Rect {
                    x: m.work_rect.x + mx,
                    y: m.work_rect.y,
                    w: w - 2 * border_width,
                    h: mh - 2 * border_width,
                },
                framecount,
                0,
            );

            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                mx += client_width(c);
            }
        } else {
            // ── stack client — full-width horizontal row ──────────────────
            animate_client_rect(
                win,
                &Rect {
                    x: tx,
                    y: ty,
                    w: m.work_rect.w - 2 * border_width,
                    h: th - 2 * border_width,
                },
                framecount,
                0,
            );

            // Advance ty only when stack rows don't fill the whole height
            // (i.e. there are multiple stack clients).
            let g = get_globals();
            if let Some(c) = g.clients.get(&win) {
                if th != m.work_rect.h {
                    ty += client_height(c);
                }
            }
        }

        i += 1;
        c_win = next_tiled(next_client);
    }
}
