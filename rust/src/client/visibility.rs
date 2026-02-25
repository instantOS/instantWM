//! Client visibility: mapping/unmapping windows and WM_STATE transitions.
//!
//! # Responsibilities
//!
//! * [`get_state`]   – read the current `WM_STATE` property from the X server.
//!                     Used once during [`crate::client::manage`] to seed
//!                     [`crate::types::Client::is_hidden`].
//! * [`is_hidden`]   – check whether a window is minimized by reading the
//!                     cached [`crate::types::Client::is_hidden`] field.
//!                     No X11 roundtrip; call `get_state` directly if you need
//!                     the live property value.
//! * [`show_hide`]   – recursively walk the stack list, positioning visible
//!                     clients on-screen and off-screen clients off to the left.
//! * [`show`]        – unmap → animate → arrange a previously hidden client.
//! * [`hide`]        – animate → unmap → iconic-state a visible client.

use crate::animation::animate_client;
use crate::client::constants::{WM_STATE_ICONIC, WM_STATE_NORMAL};
use crate::client::geometry::{client_width, resize};
use crate::client::state::set_client_state;
use crate::contexts::WmCtx;
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::layouts::arrange;
use crate::types::Rect;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;

// ---------------------------------------------------------------------------
// WM_STATE query
// ---------------------------------------------------------------------------

/// Read the `WM_STATE` property for `win` from the X server.
///
/// Returns one of the `WM_STATE_*` constants.  Falls back to
/// [`WM_STATE_NORMAL`] when the property is absent or unreadable.
pub fn get_state(win: Window) -> i32 {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else {
        return WM_STATE_NORMAL;
    };

    let globals = get_globals();
    let Ok(cookie) = conn.get_property(
        false,
        win,
        globals.cfg.wmatom.state,
        globals.cfg.wmatom.state,
        0,
        2,
    ) else {
        return WM_STATE_NORMAL;
    };

    let Ok(reply) = cookie.reply() else {
        return WM_STATE_NORMAL;
    };

    reply
        .value32()
        .and_then(|mut it| it.next())
        .map(|v| v as i32)
        .unwrap_or(WM_STATE_NORMAL)
}

/// Returns `true` when `win` is in the minimized (iconic) state.
///
/// Reads the cached [`crate::types::Client::is_hidden`] field — no X11
/// roundtrip.  The field is seeded from the live `WM_STATE` property during
/// [`crate::client::manage`] and kept in sync by [`hide`] and [`show`].
///
/// If you need the live X11 value (e.g. before the client is fully managed),
/// call [`get_state`] directly.
#[inline]
pub fn is_hidden(win: Window) -> bool {
    get_globals()
        .clients
        .get(&win)
        .map(|c| c.is_hidden)
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Recursive show/hide pass
// ---------------------------------------------------------------------------

/// Walk the stack list starting at `win`, moving each client on- or off-screen.
///
/// Visible clients (those whose tag-set overlaps the monitor's selected tags)
/// are positioned at their stored geometry.  Invisible clients are moved
/// `2 * client_width` pixels to the left of the screen (i.e. off-screen left).
///
/// This mirrors the classic dwm `showhide` function and is called by the
/// arrange path after every layout change.
pub fn show_hide(ctx: &mut WmCtx, win: Option<Window>) {
    let current = match win {
        Some(w) => w,
        None => return,
    };

    let Some(c) = ctx.g.clients.get(&current) else {
        return;
    };

    let selected_tags = c
        .mon_id
        .and_then(|mid| ctx.g.monitor(mid))
        .map(|m| m.selected_tags())
        .unwrap_or(0);
    let is_vis = c.is_visible_on_tags(selected_tags);
    let snext = c.snext;

    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    if is_vis {
        // Move the window to its stored on-screen position.
        let (x, y) = ctx
            .g
            .clients
            .get(&current)
            .map(|c| (c.geo.x, c.geo.y))
            .unwrap_or((0, 0));
        let _ = conn.configure_window(current, &ConfigureWindowAux::new().x(x).y(y));
        let _ = conn.flush();

        // For floating or non-tiling windows, also issue a full resize so the
        // stored geometry is reflected in the X server's window extents.
        let (is_floating, is_fullscreen, is_fake_fullscreen, mon_id, w, h) = ctx
            .g
            .clients
            .get(&current)
            .map(|c| {
                (
                    c.isfloating,
                    c.is_fullscreen,
                    c.isfakefullscreen,
                    c.mon_id,
                    c.geo.w,
                    c.geo.h,
                )
            })
            .unwrap_or((false, false, false, None, 0, 0));

        let is_tiling = mon_id
            .and_then(|mid| ctx.g.monitor(mid))
            .map(|mon| mon.is_tiling_layout())
            .unwrap_or(false);

        if (!is_tiling || is_floating) && (!is_fullscreen || is_fake_fullscreen) {
            resize(ctx, current, &Rect { x, y, w, h }, false);
        }

        show_hide(ctx, snext);
    } else {
        // Recurse first so children are positioned before we move the parent.
        show_hide(ctx, snext);

        let w_val = ctx.g.clients.get(&current).map(client_width).unwrap_or(0);
        let y = ctx.g.clients.get(&current).map(|c| c.geo.y).unwrap_or(0);

        let _ = conn.configure_window(current, &ConfigureWindowAux::new().x(-2 * w_val).y(y));
        let _ = conn.flush();
    }
}

// ---------------------------------------------------------------------------
// Show (unminimize)
// ---------------------------------------------------------------------------

/// Unminimize `win`: map it, animate it sliding in from above, then arrange.
///
/// Does nothing if `win` is not currently in the iconic state.
pub fn show(ctx: &mut WmCtx, win: Window) {
    let globals = get_globals();
    let Some(client) = globals.clients.get(&win) else {
        return;
    };

    if !client.is_hidden {
        return;
    }

    let Rect { x, y, w, h } = client.geo;

    // Clear the cached flag before any redraws that might be triggered below.
    if let Some(c) = get_globals_mut().clients.get_mut(&win) {
        c.is_hidden = false;
    }

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.map_window(win);
        let _ = conn.flush();
    }

    set_client_state(ctx, win, WM_STATE_NORMAL);

    // Start the window slightly above its target position so the animation
    // slides it down into place.
    resize(ctx, win, &Rect { x, y: -50, w, h }, false);

    if let Some(ref conn) = x11.conn {
        let _ = conn.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        let _ = conn.flush();
    }

    // Animate: slide down to (x, y) from (x, -50).
    animate_client(ctx, win, &Rect { x, y, w: 0, h: 0 }, 14, 0);

    let mon_id = get_globals().clients.get(&win).and_then(|c| c.mon_id);
    if let Some(mid) = mon_id {
        arrange(ctx, Some(mid));
    }
}

// ---------------------------------------------------------------------------
// Hide (minimize)
// ---------------------------------------------------------------------------

/// Minimize `win`: animate it sliding down off-screen, unmap it, then focus
/// the next client in the stack.
///
/// Does nothing if `win` is already hidden.
pub fn hide(ctx: &mut WmCtx, win: Window) {
    let globals = get_globals();
    let Some(client) = globals.clients.get(&win) else {
        return;
    };

    if client.is_hidden {
        return;
    }

    let Rect { x, y, w, h } = client.geo;
    let mon_id = client.mon_id;
    let bh = globals.cfg.bar_height;
    let animated = globals.animated;

    if animated {
        // Animate the window sliding down toward the bar before unmapping.
        animate_client(
            ctx,
            win,
            &Rect {
                x,
                y: bh - h + 40,
                w: 0,
                h: 0,
            },
            10,
            0,
        );
    }

    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    // Suppress UnmapNotify / DestroyNotify while we unmap so the event loop
    // does not try to unmanage the client.
    let _ = conn.grab_server();

    // Temporarily remove the event mask bits that would trigger an unmanage.
    let root = get_globals().cfg.root;
    suppress_unmap_events(conn, root, win);

    let _ = conn.unmap_window(win);
    let _ = conn.flush();

    set_client_state(ctx, win, WM_STATE_ICONIC);

    // Set the cached flag now that WM_STATE is committed.
    if let Some(c) = get_globals_mut().clients.get_mut(&win) {
        c.is_hidden = true;
    }

    // Restore event masks.
    restore_event_masks(conn, root, win);

    let _ = conn.ungrab_server();
    let _ = conn.flush();

    // Keep the stored geometry intact so the window returns to the right place
    // when shown again.
    resize(ctx, win, &Rect { x, y, w, h }, false);

    let snext = get_globals().clients.get(&win).and_then(|c| c.snext);
    focus(ctx, snext);

    if let Some(mid) = mon_id {
        arrange(ctx, Some(mid));
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Clear `SUBSTRUCTURE_NOTIFY` on `root` and `STRUCTURE_NOTIFY` on `win` so
/// that the imminent `unmap_window` call does not trigger an unmanage.
fn suppress_unmap_events(conn: &x11rb::rust_connection::RustConnection, root: Window, win: Window) {
    if let Ok(cookie) = conn.get_window_attributes(root) {
        if let Ok(ra) = cookie.reply() {
            let mask =
                EventMask::from(ra.your_event_mask.bits() & !EventMask::SUBSTRUCTURE_NOTIFY.bits());
            let _ = conn
                .change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(mask));
        }
    }

    if let Ok(cookie) = conn.get_window_attributes(win) {
        if let Ok(ca) = cookie.reply() {
            let mask =
                EventMask::from(ca.your_event_mask.bits() & !EventMask::STRUCTURE_NOTIFY.bits());
            let _ = conn
                .change_window_attributes(win, &ChangeWindowAttributesAux::new().event_mask(mask));
        }
    }
}

/// Re-read and restore the event masks on `root` and `win` after an unmap.
fn restore_event_masks(conn: &x11rb::rust_connection::RustConnection, root: Window, win: Window) {
    if let Ok(cookie) = conn.get_window_attributes(root) {
        if let Ok(ra) = cookie.reply() {
            let _ = conn.change_window_attributes(
                root,
                &ChangeWindowAttributesAux::new().event_mask(ra.your_event_mask),
            );
        }
    }

    if let Ok(cookie) = conn.get_window_attributes(win) {
        if let Ok(ca) = cookie.reply() {
            let _ = conn.change_window_attributes(
                win,
                &ChangeWindowAttributesAux::new().event_mask(ca.your_event_mask),
            );
        }
    }
}
