//! Client visibility: mapping/unmapping windows and WM_STATE transitions.
//!
//! # Responsibilities
//!
//! * [`get_state`]   – read the current `WM_STATE` property from the X server.
//!                     Used once during [`crate::backend::x11::lifecycle::manage`] to seed
//!                     [`crate::types::Client::is_hidden`].
//! * [`show_hide`]   – recursively walk the stack list, positioning visible
//!                     clients on-screen and off-screen clients off to the left.
//! * [`show`]        – unmap → animate → arrange a previously hidden client.
//! * [`hide`]        – animate → unmap → iconic-state a visible client.

use crate::animation::animate_client_x11;
use crate::client::constants::{WM_STATE_ICONIC, WM_STATE_NORMAL};
use crate::client::geometry::resize_x11;
use crate::client::state::set_client_state;
use crate::contexts::{CoreCtx, WaylandCtx, WmCtx, WmCtxX11, X11Ctx};
// focus() is used via focus_soft() in this module
use crate::backend::BackendOps;
use crate::layouts::arrange;
use crate::types::{Monitor, Rect, WindowId};
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
pub fn get_state_x11(core: &CoreCtx, x11: &X11Ctx, win: WindowId) -> i32 {
    let conn = x11.conn;
    let x11_win: Window = win.into();

    let wm_state_atom = core.g.x11.wmatom.state;
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
pub fn show_hide_x11(core: &mut CoreCtx, x11: &X11Ctx) {
    let mut operations: Vec<(WindowId, Rect, bool, bool, bool, bool)> = Vec::new();

    for mon in core.g.monitors_iter_all() {
        let selected_tags = mon.selected_tags();

        for &win in &mon.clients {
            let Some(c) = core.g.clients.get(&win) else {
                continue;
            };

            let is_visible = c.is_visible_on_tags(selected_tags);
            let geo = c.geo;
            let (is_floating, is_fullscreen, is_fake_fullscreen) =
                (c.isfloating, c.is_fullscreen, c.isfakefullscreen);

            operations.push((
                win,
                geo,
                is_visible,
                is_floating,
                is_fullscreen,
                is_fake_fullscreen,
            ));
        }
    }

    for (win, geo, is_visible, is_floating, is_fullscreen, is_fake_fullscreen) in operations {
        if is_visible {
            let Rect { x, y, w, h } = geo;
            let x11_win: Window = win.into();
            let width = w.max(1) as u32;
            let height = h.max(1) as u32;
            let _ = x11.conn.configure_window(
                x11_win,
                &ConfigureWindowAux::new()
                    .x(x)
                    .y(y)
                    .width(width)
                    .height(height),
            );
            let _ = x11.conn.flush();

            let is_tiling = core.g.monitors_iter().any(|(_, m)| m.is_tiling_layout());

            if (!is_tiling || is_floating) && (!is_fullscreen || is_fake_fullscreen) {
                resize_x11(core, x11, win, &Rect { x, y, w, h }, false);
            }
        } else {
            let w_val = geo.w
                + 2 * core
                    .g
                    .clients
                    .get(&win)
                    .map(|c| c.border_width())
                    .unwrap_or(0);
            let y = geo.y;

            let x11_win: Window = win.into();
            let _ = x11.conn.configure_window(
                x11_win,
                &ConfigureWindowAux::new()
                    .x(-2 * w_val)
                    .y(y)
                    .width(geo.w as u32)
                    .height(geo.h as u32),
            );
            let _ = x11.conn.flush();
        }
    }
}

pub fn show_hide_wayland(core: &mut CoreCtx, wayland: &WaylandCtx) {
    let mut operations: Vec<(WindowId, bool)> = Vec::new();

    for mon in core.g.monitors_iter_all() {
        let selected_tags = mon.selected_tags();
        for &win in &mon.clients {
            let Some(c) = core.g.clients.get(&win) else {
                continue;
            };
            let is_visible = c.is_visible_on_tags(selected_tags) && !c.is_hidden;
            operations.push((win, is_visible));
        }
    }

    for (win, is_visible) in operations {
        if is_visible {
            wayland.backend.map_window(win);
        } else {
            wayland.backend.unmap_window(win);
        }
    }
}

pub fn show_hide(ctx: &mut crate::contexts::WmCtx) {
    match ctx {
        crate::contexts::WmCtx::X11(ctx_x11) => show_hide_x11(&mut ctx_x11.core, &ctx_x11.x11),
        crate::contexts::WmCtx::Wayland(ctx_wayland) => {
            show_hide_wayland(&mut ctx_wayland.core, &ctx_wayland.wayland)
        }
    }
}

pub fn show(ctx: &mut WmCtx, win: WindowId) {
    match ctx {
        WmCtx::X11(ctx_x11) => show_x11(ctx_x11, win),
        WmCtx::Wayland(_) => show_wayland(ctx, win),
    }
}

pub fn hide(ctx: &mut WmCtx, win: WindowId) {
    match ctx {
        WmCtx::X11(ctx_x11) => hide_x11(ctx_x11, win),
        WmCtx::Wayland(_) => hide_wayland(ctx, win),
    }
}

// ---------------------------------------------------------------------------
// Show / hide (Wayland)
// ---------------------------------------------------------------------------

fn show_wayland(ctx: &mut WmCtx, win: WindowId) {
    let is_hidden = ctx
        .g()
        .clients
        .get(&win)
        .map(|c| c.is_hidden)
        .unwrap_or(false);

    if !is_hidden {
        return;
    }

    if let Some(c) = ctx.g_mut().clients.get_mut(&win) {
        c.is_hidden = false;
    }

    ctx.backend().map_window(win);
    ctx.backend().flush();

    let monitor_id = ctx.g().clients.get(&win).and_then(|c| c.monitor_id);
    crate::focus::focus_soft(ctx, Some(win));
    if let Some(mid) = monitor_id {
        arrange(ctx, Some(mid));
    }
}

fn hide_wayland(ctx: &mut WmCtx, win: WindowId) {
    let (is_hidden, monitor_id) = match ctx.g().clients.get(&win) {
        Some(c) => (c.is_hidden, c.monitor_id),
        None => return,
    };

    if is_hidden {
        return;
    }

    ctx.backend().unmap_window(win);
    ctx.backend().flush();

    if let Some(c) = ctx.g_mut().clients.get_mut(&win) {
        c.is_hidden = true;
    }

    let snext = monitor_id.and_then(|mid| {
        ctx.g()
            .monitor(mid)
            .and_then(|m| m.stack.iter().find(|&&w| w != win).copied())
    });
    crate::focus::focus_soft(ctx, snext);

    if let Some(mid) = monitor_id {
        arrange(ctx, Some(mid));
    }
}

// ---------------------------------------------------------------------------
// Show (unminimize, X11)
// ---------------------------------------------------------------------------

/// Unminimize `win`: map it, animate it sliding in from above, then arrange.
///
/// Does nothing if `win` is not currently in the iconic state.
pub fn show_x11(ctx: &mut WmCtxX11, win: WindowId) {
    let Some(client) = ctx.core.g.clients.get(&win) else {
        return;
    };

    if !client.is_hidden {
        return;
    }

    let Rect { x, y, w, h } = client.geo;

    // Clear the cached flag before any redraws that might be triggered below.
    if let Some(c) = ctx.core.g.clients.get_mut(&win) {
        c.is_hidden = false;
    }

    let x11_win: Window = win.into();
    let _ = ctx.x11.conn.map_window(x11_win);
    let _ = ctx.x11.conn.flush();

    set_client_state(&ctx.core, &ctx.x11, win, WM_STATE_NORMAL);

    // Start the window slightly above its target position so the animation
    // slides it down into place.
    resize_x11(
        &mut ctx.core,
        &ctx.x11,
        win,
        &Rect { x, y: -50, w, h },
        false,
    );

    let _ = ctx.x11.conn.configure_window(
        x11_win,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    );
    let _ = ctx.x11.conn.flush();

    // Animate: slide down to (x, y) from (x, -50).
    animate_client_x11(
        &mut ctx.core,
        &ctx.x11,
        win,
        &Rect { x, y, w: 0, h: 0 },
        14,
        0,
    );

    let monitor_id = ctx.core.g.clients.get(&win).and_then(|c| c.monitor_id);
    if let Some(mid) = monitor_id {
        arrange(&mut WmCtx::X11(ctx.reborrow()), Some(mid));
    }
}

// ---------------------------------------------------------------------------
// Hide (minimize)
// ---------------------------------------------------------------------------

/// Minimize `win`: animate it sliding down off-screen, unmap it, then focus
/// the next client in the stack.
///
/// Does nothing if `win` is already hidden.
pub fn hide_x11(ctx: &mut WmCtxX11, win: WindowId) {
    let Some(client) = ctx.core.g.clients.get(&win) else {
        return;
    };

    if client.is_hidden {
        return;
    }

    let Rect { x, y, w, h } = client.geo;
    let monitor_id = client.monitor_id;
    let bar_height = ctx.core.g.cfg.bar_height;
    let animated = ctx.core.g.animated;

    if animated {
        // Animate the window sliding down toward the bar before unmapping.
        animate_client_x11(
            &mut ctx.core,
            &ctx.x11,
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

    let root = ctx.core.g.x11.root;
    let x11_win: Window = win.into();

    // Phase 1: grab server and suppress events
    let _ = ctx.x11.conn.grab_server();
    suppress_unmap_events(ctx.x11.conn, root, x11_win);

    // Phase 2: unmap and update state (no conn borrow needed)
    let _ = ctx.x11.conn.unmap_window(x11_win);
    let _ = ctx.x11.conn.flush();
    set_client_state(&ctx.core, &ctx.x11, win, WM_STATE_ICONIC);

    if let Some(c) = ctx.core.g.clients.get_mut(&win) {
        c.is_hidden = true;
    }

    // Phase 3: restore events and ungrab
    restore_event_masks(ctx.x11.conn, root, x11_win);
    let _ = ctx.x11.conn.ungrab_server();
    let _ = ctx.x11.conn.flush();

    // Keep the stored geometry intact so the window returns to the right place
    // when shown again.
    resize_x11(&mut ctx.core, &ctx.x11, win, &Rect { x, y, w, h }, false);

    let snext = monitor_id.and_then(|mid| {
        ctx.core
            .g
            .monitor(mid)
            .and_then(|m| m.stack.iter().find(|&&w| w != win).copied())
    });
    crate::focus::focus_soft_x11(&mut ctx.core, &ctx.x11, snext);

    if let Some(mid) = monitor_id {
        arrange(&mut WmCtx::X11(ctx.reborrow()), Some(mid));
    }
}

pub fn calculate_yoffset(core: &CoreCtx, mon: &Monitor, current_tag: u32) -> i32 {
    let bar_height = core.g.cfg.bar_height;
    let base_offset = if mon.showbar { bar_height } else { 0 };

    for (_win, c) in mon.iter_clients(core.g.clients.map()) {
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
