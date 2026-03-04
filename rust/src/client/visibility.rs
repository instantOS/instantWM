//! Client visibility: mapping/unmapping windows and WM_STATE transitions.
//!
//! # Responsibilities
//!
//! * [`get_state`]   – read the current `WM_STATE` property from the X server.
//!                     Used once during [`crate::client::manage`] to seed
//!                     [`crate::types::Client::is_hidden`].
//! * [`show_hide`]   – recursively walk the stack list, positioning visible
//!                     clients on-screen and off-screen clients off to the left.
//! * [`show`]        – unmap → animate → arrange a previously hidden client.
//! * [`hide`]        – animate → unmap → iconic-state a visible client.

use crate::animation::animate_client;
use crate::backend::{BackendKind, BackendOps};
use crate::client::constants::{WM_STATE_ICONIC, WM_STATE_NORMAL};
use crate::client::geometry::{client_width, resize};
use crate::client::state::set_client_state;
use crate::contexts::WmCtx;
// focus() is used via focus_soft() in this module
use crate::layouts::arrange;
use crate::types::{Monitor, Rect, WindowId};
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;

// ---------------------------------------------------------------------------
// WM_STATE query
// ---------------------------------------------------------------------------

/// Read the `WM_STATE` property for `win` from the X server.
///
/// Returns one of the `WM_STATE_*` constants.  Falls back to
/// [`WM_STATE_NORMAL`] when the property is absent or unreadable.
pub fn get_state(ctx: &WmCtx, win: WindowId) -> i32 {
    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return WM_STATE_NORMAL;
    };
    let x11_win: Window = win.into();

    let wm_state_atom = ctx.g.cfg.wmatom.state;
    let Ok(cookie) = conn.get_property(false, x11_win, wm_state_atom, wm_state_atom, 0, 2) else {
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

// ---------------------------------------------------------------------------
// Recursive show/hide pass
// ---------------------------------------------------------------------------

/// Walk the client list, moving each client on- or off-screen.
///
/// Visible clients (those whose tag-set overlaps the monitor's selected tags)
/// are positioned at their stored geometry.  Invisible clients are moved
/// `2 * client_width` pixels to the left of the screen (i.e. off-screen left).
///
/// This mirrors the classic dwm `showhide` function and is called by the
/// arrange path after every layout change.
pub fn show_hide(ctx: &mut WmCtx) {
    let is_wayland = ctx.backend_kind() == BackendKind::Wayland;

    for (_id, mon) in ctx.g.monitors_iter() {
        let selected_tags = mon.selected_tags();

        for &win in &mon.clients {
            let Some(c) = ctx.g.clients.get(&win) else {
                continue;
            };

            let is_visible = c.is_visible_on_tags(selected_tags);
            let geo = c.geo;
            let (is_floating, is_fullscreen, is_fake_fullscreen) =
                (c.isfloating, c.is_fullscreen, c.isfakefullscreen);

            if is_visible {
                if is_wayland {
                    ctx.backend.map_window(win);
                } else {
                    let Rect { x, y, w, h } = geo;
                    ctx.backend.resize_window(win, Rect { x, y, w, h });
                    ctx.backend.flush();

                    let is_tiling = mon.is_tiling_layout();

                    if (!is_tiling || is_floating) && (!is_fullscreen || is_fake_fullscreen) {
                        resize(ctx, win, &Rect { x, y, w, h }, false);
                    }
                }
            } else {
                if is_wayland {
                    ctx.backend.unmap_window(win);
                } else {
                    let w_val = client_width(c);
                    let y = geo.y;

                    ctx.backend.resize_window(
                        win,
                        Rect {
                            x: -2 * w_val,
                            y,
                            w: geo.w,
                            h: geo.h,
                        },
                    );
                    ctx.backend.flush();
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Show (unminimize)
// ---------------------------------------------------------------------------

/// Unminimize `win`: map it, animate it sliding in from above, then arrange.
///
/// Does nothing if `win` is not currently in the iconic state.
pub fn show(ctx: &mut WmCtx, win: WindowId) {
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    let Some(client) = ctx.g.clients.get(&win) else {
        return;
    };

    if !client.is_hidden {
        return;
    }

    let Rect { x, y, w, h } = client.geo;

    // Clear the cached flag before any redraws that might be triggered below.
    if let Some(c) = ctx.g.clients.get_mut(&win) {
        c.is_hidden = false;
    }

    ctx.backend.map_window(win);
    ctx.backend.flush();

    set_client_state(ctx, win, WM_STATE_NORMAL);

    // Start the window slightly above its target position so the animation
    // slides it down into place.
    resize(ctx, win, &Rect { x, y: -50, w, h }, false);

    ctx.backend.raise_window(win);
    ctx.backend.flush();

    // Animate: slide down to (x, y) from (x, -50).
    animate_client(ctx, win, &Rect { x, y, w: 0, h: 0 }, 14, 0);

    let monitor_id = ctx.g.clients.get(&win).and_then(|c| c.monitor_id);
    if let Some(mid) = monitor_id {
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
pub fn hide(ctx: &mut WmCtx, win: WindowId) {
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    let Some(client) = ctx.g.clients.get(&win) else {
        return;
    };

    if client.is_hidden {
        return;
    }

    let Rect { x, y, w, h } = client.geo;
    let monitor_id = client.monitor_id;
    let bar_height = ctx.g.cfg.bar_height;
    let animated = ctx.g.animated;

    if animated {
        // Animate the window sliding down toward the bar before unmapping.
        animate_client(
            ctx,
            win,
            &Rect {
                x,
                y: bar_height - h + 40,
                w: 0,
                h: 0,
            },
            10,
            0,
        );
    }

    let root = ctx.g.cfg.root;
    let has_x11 = ctx.x11_conn().is_some();
    let x11_win: Window = win.into();

    if has_x11 {
        // Phase 1: grab server and suppress events (borrows conn briefly)
        if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
            let _ = conn.grab_server();
            suppress_unmap_events(conn, root, x11_win);
        }
    }

    // Phase 2: unmap and update state (no conn borrow needed)
    ctx.backend.unmap_window(win);
    ctx.backend.flush();
    set_client_state(ctx, win, WM_STATE_ICONIC);

    if let Some(c) = ctx.g.clients.get_mut(&win) {
        c.is_hidden = true;
    }

    if has_x11 {
        // Phase 3: restore events and ungrab (borrows conn briefly)
        if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
            restore_event_masks(conn, root, x11_win);
            let _ = conn.ungrab_server();
        }
        ctx.backend.flush();
    }

    // Keep the stored geometry intact so the window returns to the right place
    // when shown again.
    resize(ctx, win, &Rect { x, y, w, h }, false);

    let snext = monitor_id.and_then(|mid| {
        ctx.g
            .monitor(mid)
            .and_then(|m| m.stack.iter().find(|&&w| w != win).copied())
    });
    crate::focus::focus_soft(ctx, snext);

    if let Some(mid) = monitor_id {
        arrange(ctx, Some(mid));
    }
}

pub fn calculate_yoffset_ctx(ctx: &WmCtx, mon: &Monitor, current_tag: u32) -> i32 {
    let bar_height = ctx.g.cfg.bar_height;
    let base_offset = if mon.showbar { bar_height } else { 0 };

    for (_win, c) in mon.iter_clients(ctx.g.clients.map()) {
        if (c.tags & (1 << (current_tag - 1))) != 0 && c.is_true_fullscreen() {
            return 0;
        }
    }

    base_offset
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
