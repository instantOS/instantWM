#![allow(dead_code)]
//! Stacking layout algorithms: deck, bottom_stack, and bstackhoriz.
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
//! ## `bottom_stack` — horizontal master row, vertical stack columns
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

use crate::animation::animate_client;
use crate::client::{client_height, client_width, next_tiled, resize};
use crate::constants::animation::{BORDER_MULTIPLIER, DEFAULT_FRAME_COUNT, FAST_FRAME_COUNT};
use crate::contexts::WmCtx;
use crate::layouts::query::{count_tiled_clients, framecount_for_layout};
use crate::types::{Monitor, Rect};
use std::cmp::min;

// ── deck ─────────────────────────────────────────────────────────────────────

/// Deck layout.
///
/// The master column is split vertically among the first `nmaster` clients.
/// All stack clients are placed on top of each other in the remaining area —
/// only the topmost is visible, giving a card-deck feel.
pub fn deck(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    // ── count tiled clients ───────────────────────────────────────────────
    let n = count_tiled_clients(ctx, m);

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
    let mut master_column_offset: u32 = 0; // running y-offset inside master column
    let mut i: u32 = 0;
    let mut current_window = m
        .clients
        .first()
        .copied()
        .and_then(|w| next_tiled(ctx, Some(w)));

    while let Some(win) = current_window {
        let border_width = ctx
            .g
            .clients
            .get(&win)
            .map(|c| c.border_width())
            .unwrap_or(0);

        if i < m.nmaster as u32 {
            // ── master client — animated vertical split ───────────────────
            let h = (m.work_rect.h - master_column_offset as i32)
                / (min(n, m.nmaster as u32) - i) as i32;
            resize(
                ctx,
                win,
                &Rect {
                    x: m.work_rect.x,
                    y: m.work_rect.y + master_column_offset as i32,
                    w: mw as i32 - BORDER_MULTIPLIER * border_width,
                    h: h - BORDER_MULTIPLIER * border_width,
                },
                false,
            );

            if let Some(c) = ctx.g.clients.get(&win) {
                master_column_offset += client_height(c) as u32;
            }
        } else {
            // ── stack client — all overlap in the same rect ───────────────
            resize(
                ctx,
                win,
                &Rect {
                    x: m.work_rect.x + mw as i32,
                    y: m.work_rect.y,
                    w: m.work_rect.w - mw as i32 - BORDER_MULTIPLIER * border_width,
                    h: m.work_rect.h - BORDER_MULTIPLIER * border_width,
                },
                false,
            );
        }

        i += 1;
        current_window = next_tiled(ctx, current_window);
    }
}

// ── bottom_stack ───────────────────────────────────────────────────────────────

/// Bottom-stack layout.
///
/// The first `nmaster` clients share a horizontal master row at the top.
/// Remaining clients are divided into equal-width vertical columns below.
pub fn bottom_stack(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    let framecount = framecount_for_layout(ctx.g, 4, FAST_FRAME_COUNT, DEFAULT_FRAME_COUNT);

    // ── count tiled clients ───────────────────────────────────────────────
    let n = count_tiled_clients(ctx, m);

    if n == 0 {
        return;
    }

    // ── geometry ──────────────────────────────────────────────────────────
    // mh  — master row height
    // tw  — width of each stack column
    // stack_y_offset  — top-y of the stack row
    let (mh, tw, stack_y_offset) = if n > m.nmaster as u32 {
        let mh = if m.nmaster > 0 {
            (m.mfact * m.work_rect.h as f32) as i32
        } else {
            0
        };
        let tw = m.work_rect.w / (n - m.nmaster as u32) as i32;
        let stack_y_offset = m.work_rect.y + mh;
        (mh, tw, stack_y_offset)
    } else {
        (m.work_rect.h, m.work_rect.w, m.work_rect.y)
    };

    // ── place each client ─────────────────────────────────────────────────
    let mut master_row_offset: i32 = 0; // running x-offset inside master row
    let mut tx: i32 = m.work_rect.x; // running x-offset inside stack row
    let mut i: u32 = 0;
    let mut current_window = m
        .clients
        .first()
        .copied()
        .and_then(|w| next_tiled(ctx, Some(w)));

    while let Some(win) = current_window {
        let border_width = ctx
            .g
            .clients
            .get(&win)
            .map(|c| c.border_width())
            .unwrap_or(0);

        if i < m.nmaster as u32 {
            // ── master client — horizontal slice of the top row ───────────
            let w = (m.work_rect.w - master_row_offset) / (min(n, m.nmaster as u32) - i) as i32;
            animate_client(
                ctx,
                win,
                &Rect {
                    x: m.work_rect.x + master_row_offset,
                    y: m.work_rect.y,
                    w: w - BORDER_MULTIPLIER * border_width,
                    h: mh - BORDER_MULTIPLIER * border_width,
                },
                framecount,
                0,
            );

            if let Some(c) = ctx.g.clients.get(&win) {
                master_row_offset += client_width(c);
            }
        } else {
            // ── stack client — column in the bottom row ───────────────────
            let h = m.work_rect.h - mh;
            animate_client(
                ctx,
                win,
                &Rect {
                    x: tx,
                    y: stack_y_offset,
                    w: tw - BORDER_MULTIPLIER * border_width,
                    h: h - BORDER_MULTIPLIER * border_width,
                },
                framecount,
                0,
            );

            if let Some(c) = ctx.g.clients.get(&win) {
                if tw != m.work_rect.w {
                    tx += client_width(c);
                }
            }
        }

        i += 1;
        current_window = next_tiled(ctx, current_window);
    }
}

// ── bstackhoriz ───────────────────────────────────────────────────────────────

/// Horizontal bottom-stack layout.
///
/// Like [`bottom_stack`] but stack clients are arranged as horizontal rows rather
/// than vertical columns — each stack client spans the full work width.
pub fn bstackhoriz(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    let framecount = framecount_for_layout(ctx.g, 4, FAST_FRAME_COUNT, DEFAULT_FRAME_COUNT);

    // ── count tiled clients ───────────────────────────────────────────────
    let n = count_tiled_clients(ctx, m);

    if n == 0 {
        return;
    }

    // ── geometry ──────────────────────────────────────────────────────────
    // mh  — master row height
    // th  — height of each stack row
    // stack_y_offset  — top-y of the first stack row (mutable, advances per client)
    let (mh, th, mut stack_y_offset) = if n > m.nmaster as u32 {
        let mh = if m.nmaster > 0 {
            (m.mfact * m.work_rect.h as f32) as i32
        } else {
            0
        };
        let th = (m.work_rect.h - mh) / (n - m.nmaster as u32) as i32;
        let stack_y_offset = m.work_rect.y + mh;
        (mh, th, stack_y_offset)
    } else {
        (m.work_rect.h, m.work_rect.h, m.work_rect.y)
    };

    // ── place each client ─────────────────────────────────────────────────
    let mut master_row_offset: i32 = 0; // running x-offset inside master row
    let tx: i32 = m.work_rect.x;
    let mut i: u32 = 0;
    let mut current_window = m
        .clients
        .first()
        .copied()
        .and_then(|w| next_tiled(ctx, Some(w)));

    while let Some(win) = current_window {
        let border_width = ctx
            .g
            .clients
            .get(&win)
            .map(|c| c.border_width())
            .unwrap_or(0);

        if i < m.nmaster as u32 {
            // ── master client — horizontal slice of the top row ───────────
            let w = (m.work_rect.w - master_row_offset) / (min(n, m.nmaster as u32) - i) as i32;
            animate_client(
                ctx,
                win,
                &Rect {
                    x: m.work_rect.x + master_row_offset,
                    y: m.work_rect.y,
                    w: w - BORDER_MULTIPLIER * border_width,
                    h: mh - BORDER_MULTIPLIER * border_width,
                },
                framecount,
                0,
            );

            if let Some(c) = ctx.g.clients.get(&win) {
                master_row_offset += client_width(c);
            }
        } else {
            // ── stack client — full-width horizontal row ──────────────────
            animate_client(
                ctx,
                win,
                &Rect {
                    x: tx,
                    y: stack_y_offset,
                    w: m.work_rect.w - BORDER_MULTIPLIER * border_width,
                    h: th - BORDER_MULTIPLIER * border_width,
                },
                framecount,
                0,
            );

            // Advance stack_y_offset only when stack rows don't fill the whole height
            // (i.e. there are multiple stack clients).
            if let Some(c) = ctx.g.clients.get(&win) {
                if th != m.work_rect.h {
                    stack_y_offset += client_height(c);
                }
            }
        }

        i += 1;
        current_window = next_tiled(ctx, current_window);
    }
}
