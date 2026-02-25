//! Floating layout and snap-position geometry helpers.
//!
//! ## Overview
//!
//! In the floating layout every client is responsible for its own position.
//! The role of [`floatl`] is therefore minimal: it temporarily disables
//! animation, applies any pending *snap positions* (e.g. half-screen left,
//! quarter top-right) to clients that have one set, restacks the windows in
//! the correct order, and raises the selected client to the top.
//!
//! ## Snap positions
//!
//! A snap position is stored on each client as a [`SnapPosition`] enum
//! variant.  When a floating client is dragged to a screen edge the WM sets
//! `client.snapstatus`; [`floatl`] then calls [`apply_snap_for_window`] to
//! compute and apply the corresponding geometry.
//!
//! ```text
//! ┌──────────────────────────────────┐
//! │  TopLeft   │   Top   │ TopRight  │
//! ├────────────┼─────────┼───────────┤
//! │    Left    │ (none)  │   Right   │
//! ├────────────┼─────────┼───────────┤
//! │ BottomLeft │ Bottom  │BotRight   │
//! └──────────────────────────────────┘
//!                   ↑ Maximized fills the whole work area
//! ```
//!
//! ## `save_floating`
//!
//! A small helper that copies `client.geo` into `client.float_geo`.  It is
//! used here to checkpoint a floating client's position before the overview
//! layout moves it, so the original position can be restored later.

use crate::client::resize;
use crate::contexts::WmCtx;
use crate::types::{Monitor, Rect, SnapPosition};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

// ── floatl ────────────────────────────────────────────────────────────────────

/// Floating layout arrange function.
///
/// Called by the [`FloatingLayout`](crate::layouts::FloatingLayout),
/// [`VertLayout`](crate::layouts::VertLayout), and
/// [`HorizLayout`](crate::layouts::HorizLayout) impls — all of which leave
/// clients at their self-managed positions but still need snap geometry
/// enforced and the window stack sorted.
pub fn floatl(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    let selected = m.selected_tags();
    // Disable animation for the duration of this arrange pass — floating
    // windows should snap into their positions instantly.
    let animation_was_on = ctx.g.animated;
    if animation_was_on {
        ctx.g.animated = false;
    }

    // ── apply pending snap positions ──────────────────────────────────────
    let mut c_win = m.clients;
    while let Some(win) = c_win {
        let c = match ctx.g.clients.get(&win) {
            Some(c) => c,
            None => break,
        };

        if !c.is_visible_on_tags(selected) {
            c_win = c.next;
            continue;
        }

        let snapstatus = c.snapstatus;
        let next_client = c.next;

        if snapstatus != SnapPosition::None {
            apply_snap_for_window(ctx, win, m);
        }

        c_win = next_client;
    }

    // Raise the selected window to the top of the Z-order so it is not
    // accidentally obscured by a tiled window placed above it by the compositor.
    if let Some(sel_win) = m.sel {
        if let Some(ref conn) = ctx.x11.conn {
            let _ = configure_window(
                conn,
                sel_win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
            let _ = conn.flush();
        }
    }

    // Restore animation flag.
    if animation_was_on {
        ctx.g.animated = true;
    }
}

// ── apply_snap_for_window ─────────────────────────────────────────────────────

/// Compute and apply the geometry dictated by a client's [`SnapPosition`].
///
/// This is a pure geometry function: it reads `client.snapstatus` and
/// `client.border_width`, derives the target `Rect` from the monitor's
/// `work_rect`, and calls [`resize`].  It does *not* modify `snapstatus`.
///
/// Returns immediately if `snapstatus` is [`SnapPosition::None`] or the
/// client window is not found.
pub fn apply_snap_for_window(ctx: &mut WmCtx<'_>, win: Window, m: &Monitor) {
    let c = match ctx.g.clients.get(&win) {
        Some(c) => c,
        None => return,
    };

    let snapstatus = c.snapstatus;
    let bw = c.border_width; // border width in pixels
    let wr = &m.work_rect; // shorthand

    // Half-dimensions, pre-computed to keep match arms readable.
    let half_w = wr.w / 2;
    let half_h = wr.h / 2;

    let (x, y, w, h) = match snapstatus {
        // ── half-screen positions ─────────────────────────────────────────
        SnapPosition::Top => (wr.x, wr.y, wr.w - 2 * bw, half_h - 2 * bw),
        SnapPosition::Bottom => (wr.x, wr.y + half_h, wr.w - 2 * bw, half_h - 2 * bw),
        SnapPosition::Left => (wr.x, wr.y, half_w - 2 * bw, wr.h - 2 * bw),
        SnapPosition::Right => (wr.x + half_w, wr.y, half_w - 2 * bw, wr.h - 2 * bw),
        // ── quarter-screen (corner) positions ─────────────────────────────
        SnapPosition::TopLeft => (wr.x, wr.y, half_w - 2 * bw, half_h - 2 * bw),
        SnapPosition::TopRight => (wr.x + half_w, wr.y, half_w - 2 * bw, half_h - 2 * bw),
        SnapPosition::BottomLeft => (wr.x, wr.y + half_h, half_w - 2 * bw, half_h - 2 * bw),
        SnapPosition::BottomRight => (
            wr.x + half_w,
            wr.y + half_h,
            half_w - 2 * bw,
            half_h - 2 * bw,
        ),
        // ── full work-area maximise ───────────────────────────────────────
        SnapPosition::Maximized => (wr.x, wr.y, wr.w - 2 * bw, wr.h - 2 * bw),
        // ── no snap — nothing to do ───────────────────────────────────────
        SnapPosition::None => return,
    };

    resize(ctx, win, &Rect { x, y, w, h }, false);
}

// ── save_floating ─────────────────────────────────────────────────────────────

/// Persist the current geometry of `win` as its floating geometry.
///
/// Called before any operation that will move a floating client (such as the
/// overview layout), so the original position can be restored afterwards via
/// `restore_floating_win`.
pub fn save_floating(ctx: &mut WmCtx<'_>, win: Window) {
    if let Some(c) = ctx.g.clients.get_mut(&win) {
        c.float_geo = c.geo;
    }
}
