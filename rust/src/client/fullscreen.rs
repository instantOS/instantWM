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

use crate::animation::animate_client_x11;
use crate::client::geometry::resize_client_x11;
use crate::contexts::{CoreCtx, X11Ctx};
use crate::layouts::arrange;
use crate::types::{Rect, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

// ---------------------------------------------------------------------------
// Real fullscreen
// ---------------------------------------------------------------------------

/// Enter or exit fullscreen for `win`.
pub fn set_fullscreen_x11(core: &mut CoreCtx, x11: &X11Ctx, win: WindowId, fullscreen: bool) {
    let x11_win: Window = win.into();

    let net_wm_fullscreen = core.g.x11.netatom.wm_fullscreen;
    let net_wm_state = core.g.x11.netatom.wm_state;

    // Snapshot what we need before taking a mutable borrow.
    let client_snapshot = core.g.clients.get(&win).map(|c| {
        (
            c.is_fullscreen,
            c.isfloating,
            c.isfakefullscreen,
            c.monitor_id,
            c.oldstate,
            c.old_geo,
        )
    });

    let Some((is_fs, is_floating, is_fake_fs, monitor_id, _oldstate, old_geo)) = client_snapshot
    else {
        return;
    };

    if fullscreen && !is_fs {
        // ---- Enter fullscreen -----------------------------------------------

        // Advertise the new state via EWMH.
        let _ = x11.conn.change_property32(
            PropMode::REPLACE,
            x11_win,
            net_wm_state,
            AtomEnum::ATOM,
            &[net_wm_fullscreen],
        );

        if let Some(c) = core.g.clients.get_mut(&win) {
            c.is_fullscreen = true;
            c.oldstate = c.isfloating as i32;
        }

        core.g.clients.save_border_width(win);

        if !is_fake_fs {
            // Remove the border.
            if let Some(c) = core.g.clients.get_mut(&win) {
                c.border_width = 0;
            }

            let mon_rect = monitor_id
                .and_then(|mid| core.g.monitor(mid).map(|m| m.monitor_rect))
                .unwrap_or_default();

            // Animate the expansion only for non-floating clients (floating
            // windows just snap into place immediately).
            if !is_floating {
                animate_client_x11(core, x11, win, &mon_rect, 10, 0);
            }

            // Position and raise the window.
            let _ = x11.conn.configure_window(
                x11_win,
                &ConfigureWindowAux::new()
                    .x(mon_rect.x)
                    .y(mon_rect.y)
                    .width(mon_rect.w as u32)
                    .height(mon_rect.h as u32),
            );
            let _ = x11.conn.configure_window(
                x11_win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
            let _ = x11.conn.flush();
        }

        // Mark as floating so the layout engine leaves it alone.
        if let Some(c) = core.g.clients.get_mut(&win) {
            c.isfloating = true;
        }
    } else if !fullscreen && is_fs {
        // ---- Exit fullscreen ------------------------------------------------

        // Clear the EWMH state property.
        let _ = x11.conn.change_property32(
            PropMode::REPLACE,
            x11_win,
            net_wm_state,
            AtomEnum::ATOM,
            &[],
        );

        if let Some(c) = core.g.clients.get_mut(&win) {
            c.is_fullscreen = false;
            c.isfloating = c.oldstate != 0;
        }

        core.g.clients.restore_border_width(win);

        if !is_fake_fs {
            // Snap back to the geometry that was stored before going fullscreen.
            resize_client_x11(core, x11, win, &old_geo);
            let _ = x11.conn.flush();

            if let Some(mid) = monitor_id {
                let mut tmp = crate::contexts::WmCtx::X11(crate::contexts::WmCtxX11 {
                    core: core.reborrow(),
                    backend: crate::backend::BackendRef::from_x11(x11.conn, x11.screen_num),
                    x11: crate::contexts::X11Ctx {
                        conn: x11.conn,
                        screen_num: x11.screen_num,
                    },
                });
                arrange(&mut tmp, Some(mid));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fake fullscreen toggle
// ---------------------------------------------------------------------------

/// Toggle the "fake fullscreen" mode on the currently selected window.
pub fn toggle_fake_fullscreen_x11(core: &mut CoreCtx, x11: &X11Ctx) {
    let Some(win) = core.selected_client() else {
        return;
    };

    let (is_fullscreen, isfakefullscreen, monitor_id, old_border_width) = core
        .g
        .clients
        .get(&win)
        .map(|c| {
            (
                c.is_fullscreen,
                c.isfakefullscreen,
                c.monitor_id,
                c.old_border_width,
            )
        })
        .unwrap_or((false, false, None, 0));

    // Transitioning from fake-fullscreen → real-fullscreen: resize to fill the
    // monitor and raise the window.
    if is_fullscreen && isfakefullscreen {
        let borderpx = core.g.cfg.borderpx;

        if let Some(mid) = monitor_id {
            let mon_rect = core
                .g
                .monitor(mid)
                .map(|m| m.monitor_rect)
                .unwrap_or_default();

            resize_client_x11(
                core,
                x11,
                win,
                &Rect {
                    x: mon_rect.x + borderpx,
                    y: mon_rect.y + borderpx,
                    w: mon_rect.w - 2 * borderpx,
                    h: mon_rect.h - 2 * borderpx,
                },
            );

            let x11_win: Window = win.into();
            let _ = x11.conn.configure_window(
                x11_win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
            let _ = x11.conn.flush();
        }
    }

    // Restore the border width when leaving fake-fullscreen while still in
    // the fullscreen state (real fullscreen removes the border, so we need to
    // put it back before the layout re-runs).
    if let Some(client) = core.g.clients.get_mut(&win) {
        if client.is_fullscreen && !client.isfakefullscreen {
            client.border_width = old_border_width;
        }
        client.isfakefullscreen = !client.isfakefullscreen;
    }
}
