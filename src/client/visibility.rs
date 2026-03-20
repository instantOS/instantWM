//! Client visibility: mapping/unmapping windows and WM_STATE transitions.

use crate::animation::animate_client_x11;
use crate::backend::BackendOps;
use crate::backend::x11::X11BackendRef;
use crate::client::constants::{WM_STATE_ICONIC, WM_STATE_NORMAL};
use crate::client::geometry::resize;
use crate::client::state::set_client_state;
use crate::contexts::{CoreCtx, WmCtx, WmCtxWayland, WmCtxX11};
use crate::layouts::arrange;
use crate::types::{Rect, WindowId};
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
pub fn get_state_x11(
    _core: &CoreCtx,
    x11: &X11BackendRef,
    wm_state_atom: u32,
    win: WindowId,
) -> i32 {
    let conn = x11.conn;
    let x11_win: Window = win.into();
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
pub fn show_hide_x11(ctx: &mut WmCtxX11<'_>) {
    // First pass: collect visibility data to avoid borrow issues
    let mut operations: Vec<(WindowId, Rect, bool, bool, bool, bool)> = Vec::new();

    for mon in ctx.core.globals().monitors_iter_all() {
        let selected_tags = mon.selected_tags();

        for (win, c) in mon.iter_clients(ctx.core.globals().clients.map()) {
            let is_visible = c.is_visible_on_tags(selected_tags) && !c.is_hidden;
            let geo = c.geo;
            let (is_floating, is_fullscreen, is_fake_fullscreen) =
                (c.is_floating, c.is_fullscreen, c.isfakefullscreen);

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

    // Second pass: apply visibility changes
    for (win, geo, is_visible, is_floating, is_fullscreen, is_fake_fullscreen) in operations {
        if is_visible {
            let Rect { x, y, w, h } = geo;
            let x11_win: Window = win.into();
            let width = w.max(1) as u32;
            let height = h.max(1) as u32;
            let _ = ctx.x11.conn.configure_window(
                x11_win,
                &ConfigureWindowAux::new()
                    .x(x)
                    .y(y)
                    .width(width)
                    .height(height),
            );
            let _ = ctx.x11.conn.flush();

            let is_tiling = ctx
                .core
                .globals()
                .monitors_iter()
                .any(|(_, m)| m.is_tiling_layout());

            if (!is_tiling || is_floating) && (!is_fullscreen || is_fake_fullscreen) {
                let mut tmp_ctx = WmCtx::X11(ctx.reborrow());
                resize(&mut tmp_ctx, win, &Rect { x, y, w, h }, false);
            }
        } else {
            let w_val = geo.w
                + 2 * ctx
                    .core
                    .globals()
                    .clients
                    .get(&win)
                    .map(|c| c.border_width)
                    .unwrap_or(0);
            let y = geo.y;

            let x11_win: Window = win.into();
            let _ = ctx.x11.conn.configure_window(
                x11_win,
                &ConfigureWindowAux::new()
                    .x(-2 * w_val)
                    .y(y)
                    .width(geo.w as u32)
                    .height(geo.h as u32),
            );
            let _ = ctx.x11.conn.flush();
        }
    }
}

pub fn show_hide_wayland(ctx: &mut WmCtxWayland<'_>) {
    // First pass: collect visibility data
    let mut operations: Vec<(WindowId, bool)> = Vec::new();

    for mon in ctx.core.globals().monitors_iter_all() {
        let selected_tags = mon.selected_tags();
        for (win, c) in mon.iter_clients(ctx.core.globals().clients.map()) {
            let is_visible = c.is_visible_on_tags(selected_tags) && !c.is_hidden;
            operations.push((win, is_visible));
        }
    }

    // Second pass: apply visibility changes
    for (win, is_visible) in operations {
        if is_visible {
            ctx.wayland.backend.map_window(win);
        } else {
            ctx.wayland.backend.unmap_window(win);
        }
    }
}

pub fn show_hide(ctx: &mut crate::contexts::WmCtx) {
    match ctx {
        crate::contexts::WmCtx::X11(ctx_x11) => show_hide_x11(ctx_x11),
        crate::contexts::WmCtx::Wayland(ctx_wayland) => show_hide_wayland(ctx_wayland),
    }
}

pub fn show(ctx: &mut WmCtx, win: WindowId) {
    let monitor_id = if let Some(c) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
        if !c.is_hidden {
            return;
        }
        c.is_hidden = false;
        c.monitor_id
    } else {
        return;
    };

    if let WmCtx::X11(ctx_x11) = ctx {
        show_x11(ctx_x11, win);
    }
    // On Wayland, map_window is not called here directly. show_hide_wayland
    // (called inside arrange below) checks !is_hidden and calls map_window
    // itself, so the window reappears as a side-effect of the arrange pass.

    crate::focus::focus_soft(ctx, Some(win));
    arrange(ctx, Some(monitor_id));
}

pub fn hide(ctx: &mut WmCtx, win: WindowId) {
    let monitor_id = if let Some(c) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
        if c.is_hidden {
            return;
        }
        let mid = c.monitor_id;

        match ctx {
            WmCtx::X11(ctx_x11) => {
                hide_x11(ctx_x11, win);
            }
            WmCtx::Wayland(_) => {
                ctx.backend().unmap_window(win);
                ctx.backend().flush();
            }
        }

        if let Some(c_mut) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
            c_mut.is_hidden = true;
        }

        mid
    } else {
        return;
    };

    let snext = ctx
        .core()
        .globals()
        .monitor(monitor_id)
        .and_then(|m| m.stack.iter().find(|&&w| w != win).copied());
    crate::focus::focus_soft(ctx, snext);
    arrange(ctx, Some(monitor_id));
}

// ---------------------------------------------------------------------------
// Show (unminimize, X11)
// ---------------------------------------------------------------------------

/// X11-specific mechanics for unminimizing `win`.
///
/// Called by [`show`] after it has cleared `is_hidden`. Responsible only for
/// the X11-specific work: mapping the window, WM_STATE, slide-in animation.
/// Guards, focus, and arrange are handled by the caller.
fn show_x11(ctx: &mut WmCtxX11<'_>, win: WindowId) {
    let Rect { x, y, w, h } = match ctx.core.globals().clients.get(&win) {
        Some(c) => c.geo,
        None => return,
    };

    let x11_win: Window = win.into();
    let _ = ctx.x11.conn.map_window(x11_win);
    let _ = ctx.x11.conn.flush();

    set_client_state(&ctx.core, &ctx.x11, ctx.x11_runtime, win, WM_STATE_NORMAL);

    // Start the window slightly above its target position so the animation
    // slides it down into place.
    {
        let mut tmp_ctx = WmCtx::X11(ctx.reborrow());
        resize(&mut tmp_ctx, win, &Rect { x, y: -50, w, h }, false);
    }

    let _ = ctx.x11.conn.configure_window(
        x11_win,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    );
    let _ = ctx.x11.conn.flush();

    // Animate: slide down to (x, y) from (x, -50).
    animate_client_x11(ctx, win, &Rect { x, y, w: 0, h: 0 }, 14, 0);
}

// ---------------------------------------------------------------------------
// Hide (minimize, X11)
// ---------------------------------------------------------------------------

/// X11-specific mechanics for minimizing `win`.
///
/// Called by [`hide`] before it sets `is_hidden`. Responsible only for the
/// X11-specific work: slide-down animation, server grab, unmap, WM_STATE,
/// and geometry preservation. Guards, focus, and arrange are handled by the
/// caller.
fn hide_x11(ctx: &mut WmCtxX11<'_>, win: WindowId) {
    let Rect { x, y, w, h } = match ctx.core.globals().clients.get(&win) {
        Some(c) => c.geo,
        None => return,
    };
    let bar_height = ctx.core.globals().cfg.bar_height;
    let animated = ctx.core.globals().behavior.animated;

    if animated {
        // Animate the window sliding down toward the bar before unmapping.
        animate_client_x11(
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

    let root = ctx.x11_runtime.root;
    let x11_win: Window = win.into();

    {
        let _grab = crate::backend::x11::ServerGrab::new(ctx.x11.conn);
        suppress_unmap_events(ctx.x11.conn, root, x11_win);

        let _ = ctx.x11.conn.unmap_window(x11_win);
        let _ = ctx.x11.conn.flush();
        set_client_state(&ctx.core, &ctx.x11, ctx.x11_runtime, win, WM_STATE_ICONIC);

        restore_event_masks(ctx.x11.conn, root, x11_win);
    }

    // Keep the stored geometry intact so the window returns to the right place
    // when shown again.
    {
        let mut tmp_ctx = WmCtx::X11(ctx.reborrow());
        resize(&mut tmp_ctx, win, &Rect { x, y, w, h }, false);
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Clear `SUBSTRUCTURE_NOTIFY` on `root` and `STRUCTURE_NOTIFY` on `win` so
/// that the imminent `unmap_window` call does not trigger an unmanage.
fn suppress_unmap_events(conn: &x11rb::rust_connection::RustConnection, root: Window, win: Window) {
    if let Ok(cookie) = conn.get_window_attributes(root)
        && let Ok(ra) = cookie.reply()
    {
        let mask =
            EventMask::from(ra.your_event_mask.bits() & !EventMask::SUBSTRUCTURE_NOTIFY.bits());
        let _ =
            conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(mask));
    }

    if let Ok(cookie) = conn.get_window_attributes(win)
        && let Ok(ca) = cookie.reply()
    {
        let mask = EventMask::from(ca.your_event_mask.bits() & !EventMask::STRUCTURE_NOTIFY.bits());
        let _ =
            conn.change_window_attributes(win, &ChangeWindowAttributesAux::new().event_mask(mask));
    }
}

/// Re-read and restore the event masks on `root` and `win` after an unmap.
fn restore_event_masks(conn: &x11rb::rust_connection::RustConnection, root: Window, win: Window) {
    if let Ok(cookie) = conn.get_window_attributes(root)
        && let Ok(ra) = cookie.reply()
    {
        let _ = conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(ra.your_event_mask),
        );
    }

    if let Ok(cookie) = conn.get_window_attributes(win)
        && let Ok(ca) = cookie.reply()
    {
        let _ = conn.change_window_attributes(
            win,
            &ChangeWindowAttributesAux::new().event_mask(ca.your_event_mask),
        );
    }
}
