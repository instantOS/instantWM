//! Fullscreen and fake-fullscreen state management.
//!
//! # Responsibilities
//!
//! * [`set_fullscreen`]          – enter/exit real fullscreen, updating
//!                                 `_NET_WM_STATE` and animating the transition.
//! * [`toggle_fake_fullscreen`]  – toggle "fake" fullscreen (window fills the
//!                                 monitor but still participates in the layout).
//! * [`save_border_width`]       – snapshot the current border width before
//!                                 entering fullscreen.
//! * [`restore_border_width`]    – reinstate the saved border width on exit.
//!
//! ## Real vs. fake fullscreen
//!
//! *Real* fullscreen (`is_fullscreen = true`, `isfakefullscreen = false`):
//! the border is removed, the window is raised above everything else, and it
//! is resized to exactly the monitor rectangle.
//!
//! *Fake* fullscreen (`is_fullscreen = true`, `isfakefullscreen = true`):
//! the `_NET_WM_STATE_FULLSCREEN` atom is set (so the application thinks it is
//! fullscreen) but the window remains in the normal layout stack with its
//! border intact.

use crate::animation::animate_client;
use crate::backend::BackendKind;
use crate::client::geometry::resize_client;
use crate::contexts::WmCtx;
use crate::globals::{get_globals, get_globals_mut};
use crate::layouts::arrange;
use crate::types::{Rect, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

// ---------------------------------------------------------------------------
// Border-width save / restore
// ---------------------------------------------------------------------------

/// Copy `client.border_width` → `client.old_border_width`.
///
/// Call this just before entering fullscreen (or stripping the border for a
/// single-client tiling layout) so that [`restore_border_width`] can put the
/// border back on exit.
///
/// **Guard:** if `border_width` is already 0 the save is skipped — we must
/// never clobber a previously saved non-zero value with 0, or restoring would
/// silently become a no-op.  This matches the C `savebw` implementation.
pub fn save_border_width(win: WindowId) {
    let globals = get_globals_mut();
    save_border_width_in(&mut globals.clients, win);
}

/// Save border width using the active WM context.
#[inline]
pub fn save_border_width_ctx(ctx: &mut WmCtx, win: WindowId) {
    save_border_width_in(&mut ctx.g.clients, win);
}

/// Save border width using an explicit client map reference.
pub(crate) fn save_border_width_in(
    clients: &mut std::collections::HashMap<WindowId, crate::types::Client>,
    win: WindowId,
) {
    if let Some(client) = clients.get_mut(&win) {
        if client.border_width != 0 {
            client.old_border_width = client.border_width;
        }
    }
}

/// Copy `client.old_border_width` → `client.border_width`.
///
/// **Guard:** if `old_border_width` is 0 (i.e. was never saved, or the
/// window genuinely had no border) the restore is skipped — unconditionally
/// writing 0 back would remove the border from windows that were managed
/// without ever going through the strip path.  This matches the C
/// `restore_border_width` implementation.
pub fn restore_border_width(win: WindowId) {
    let globals = get_globals_mut();
    restore_border_width_in(&mut globals.clients, win);
}

/// Restore border width using the active WM context.
#[inline]
pub fn restore_border_width_ctx(ctx: &mut WmCtx, win: WindowId) {
    restore_border_width_in(&mut ctx.g.clients, win);
}

/// Restore border width using an explicit client map reference.
pub(crate) fn restore_border_width_in(
    clients: &mut std::collections::HashMap<WindowId, crate::types::Client>,
    win: WindowId,
) {
    if let Some(client) = clients.get_mut(&win) {
        if client.old_border_width != 0 {
            client.border_width = client.old_border_width;
        }
    }
}

// ---------------------------------------------------------------------------
// Real fullscreen
// ---------------------------------------------------------------------------

/// Enter or exit fullscreen for `win`.
///
/// * `fullscreen = true`  – removes the border, raises the window, resizes it
///                          to the monitor rectangle, and sets
///                          `_NET_WM_STATE_FULLSCREEN`.
/// * `fullscreen = false` – restores the saved geometry and border, clears
///                          the `_NET_WM_STATE` property, and re-arranges the
///                          monitor.
///
/// When the client has `isfakefullscreen` enabled, geometry/border changes are
/// skipped – only the EWMH property is toggled so the application remains happy
/// while the window keeps participating in the tiling layout.
pub fn set_fullscreen(ctx: &mut WmCtx, win: WindowId, fullscreen: bool) {
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    let x11_win: Window = win.into();

    let net_wm_fullscreen = ctx.g.cfg.netatom.wm_fullscreen;
    let net_wm_state = ctx.g.cfg.netatom.wm_state;

    // Snapshot what we need before taking a mutable borrow.
    let client_snapshot = ctx.g.clients.get(&win).map(|c| {
        (
            c.is_fullscreen,
            c.isfloating,
            c.isfakefullscreen,
            c.mon_id,
            c.oldstate,
            c.old_geo,
        )
    });

    let Some((is_fs, is_floating, is_fake_fs, mon_id, _oldstate, old_geo)) = client_snapshot else {
        return;
    };

    if fullscreen && !is_fs {
        // ---- Enter fullscreen -----------------------------------------------

        // Advertise the new state via EWMH.
        if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
            let _ = conn.change_property32(
                PropMode::REPLACE,
                x11_win,
                net_wm_state,
                AtomEnum::ATOM,
                &[net_wm_fullscreen],
            );
        }

        if let Some(c) = ctx.g.clients.get_mut(&win) {
            c.is_fullscreen = true;
            c.oldstate = c.isfloating as i32;
        }

        save_border_width_in(&mut ctx.g.clients, win);

        if !is_fake_fs {
            // Remove the border.
            if let Some(c) = ctx.g.clients.get_mut(&win) {
                c.border_width = 0;
            }

            let mon_rect = mon_id
                .and_then(|mid| ctx.g.monitor(mid).map(|m| m.monitor_rect))
                .unwrap_or_default();

            // Animate the expansion only for non-floating clients (floating
            // windows just snap into place immediately).
            if !is_floating {
                animate_client(ctx, win, &mon_rect, 10, 0);
            }

            // Position and raise the window.
            if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
                let _ = conn.configure_window(
                    x11_win,
                    &ConfigureWindowAux::new()
                        .x(mon_rect.x)
                        .y(mon_rect.y)
                        .width(mon_rect.w as u32)
                        .height(mon_rect.h as u32),
                );
                let _ = conn.configure_window(
                    x11_win,
                    &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                );
                let _ = conn.flush();
            }
        }

        // Mark as floating so the layout engine leaves it alone.
        if let Some(c) = ctx.g.clients.get_mut(&win) {
            c.isfloating = true;
        }
    } else if !fullscreen && is_fs {
        // ---- Exit fullscreen ------------------------------------------------

        // Clear the EWMH state property.
        if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
            let _ = conn.change_property32(
                PropMode::REPLACE,
                x11_win,
                net_wm_state,
                AtomEnum::ATOM,
                &[],
            );
        }

        if let Some(c) = ctx.g.clients.get_mut(&win) {
            c.is_fullscreen = false;
            c.isfloating = c.oldstate != 0;
        }

        restore_border_width_in(&mut ctx.g.clients, win);

        if !is_fake_fs {
            // Snap back to the geometry that was stored before going fullscreen.
            resize_client(ctx, win, &old_geo);

            if let Some(mid) = mon_id {
                arrange(ctx, Some(mid));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fake fullscreen toggle
// ---------------------------------------------------------------------------

/// Toggle the "fake fullscreen" mode on the currently selected window.
///
/// When switching *out* of fake fullscreen (i.e. `isfakefullscreen` was `true`
/// and `is_fullscreen` is still `true`), the window is resized to fill the
/// monitor rectangle with the current border and raised above everything else.
///
/// The `isfakefullscreen` flag itself is flipped at the end regardless of the
/// current state.
pub fn toggle_fake_fullscreen(ctx: &mut WmCtx) {
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    let Some(win) = ctx.g.selected_win() else {
        return;
    };

    let (is_fullscreen, isfakefullscreen, mon_id, old_border_width) = ctx
        .g
        .clients
        .get(&win)
        .map(|c| {
            (
                c.is_fullscreen,
                c.isfakefullscreen,
                c.mon_id,
                c.old_border_width,
            )
        })
        .unwrap_or((false, false, None, 0));

    // Transitioning from fake-fullscreen → real-fullscreen: resize to fill the
    // monitor and raise the window.
    if is_fullscreen && isfakefullscreen {
        let borderpx = ctx.g.cfg.borderpx;

        if let Some(mid) = mon_id {
            let mon_rect = ctx
                .g
                .monitor(mid)
                .map(|m| m.monitor_rect)
                .unwrap_or_default();

            resize_client(
                ctx,
                win,
                &Rect {
                    x: mon_rect.x + borderpx,
                    y: mon_rect.y + borderpx,
                    w: mon_rect.w - 2 * borderpx,
                    h: mon_rect.h - 2 * borderpx,
                },
            );

            if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
                let x11_win: Window = win.into();
                let _ = conn.configure_window(
                    x11_win,
                    &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                );
                let _ = conn.flush();
            }
        }
    }

    // Restore the border width when leaving fake-fullscreen while still in
    // the fullscreen state (real fullscreen removes the border, so we need to
    // put it back before the layout re-runs).
    if let Some(client) = ctx.g.clients.get_mut(&win) {
        if client.is_fullscreen && !client.isfakefullscreen {
            client.border_width = old_border_width;
        }
        client.isfakefullscreen = !client.isfakefullscreen;
    }
}
