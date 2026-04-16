//! Fullscreen and fake-fullscreen state management.
//!
//! # Responsibilities
//!
//! * [`set_fullscreen`]         – enter/exit real fullscreen, updating
//!   `_NET_WM_STATE` and animating the transition.
//! * [`toggle_fake_fullscreen`] – toggle "fake" fullscreen (window fills the
//!   monitor but still participates in the layout).
//! * [`save_border_width`]      – snapshot the current border width before
//!   entering fullscreen.
//! * [`restore_border_width`]   – reinstate the saved border width on exit.
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

use crate::backend::x11::properties::{get_atom_props, write_net_wm_state_atoms};
use crate::constants::animation::EMPHASIZED_FRAME_COUNT;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::geometry::MoveResizeOptions;
use crate::layouts::{arrange, sync_monitor_z_order};
use crate::types::{Rect, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;

// ---------------------------------------------------------------------------
// Real fullscreen
// ---------------------------------------------------------------------------

/// Enter or exit fullscreen for `win`.
pub fn set_fullscreen_x11(ctx_x11: &mut WmCtxX11<'_>, win: WindowId, fullscreen: bool) {
    let x11_win: Window = win.into();
    let conn = ctx_x11.x11.conn;
    let wm_state = ctx_x11.x11_runtime.netatom.wm_state;
    let net_wm_fullscreen = ctx_x11.x11_runtime.netatom.wm_fullscreen;

    // Snapshot what we need before taking a mutable borrow.
    let client_snapshot = ctx_x11.core.client(win).map(|c| {
        (
            c.is_fullscreen,
            c.is_floating,
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

        let mut state_atoms = get_atom_props(conn, x11_win, wm_state);
        if !state_atoms.contains(&net_wm_fullscreen) {
            state_atoms.push(net_wm_fullscreen);
        }
        write_net_wm_state_atoms(conn, x11_win, wm_state, &state_atoms);

        if let Some(c) = ctx_x11.core.globals_mut().clients.get_mut(&win) {
            c.is_fullscreen = true;
            c.oldstate = c.is_floating as i32;
            c.save_border_width();

            if !is_fake_fs {
                // Remove the border.
                c.border_width = 0;
            }

            // Mark as floating so the layout engine leaves it alone.
            c.is_floating = true;
        }

        if !is_fake_fs {
            let mon_rect = ctx_x11
                .core
                .globals()
                .monitor(monitor_id)
                .map(|m| m.monitor_rect)
                .unwrap_or_default();

            // Animate the expansion only for non-floating clients (floating
            // windows just snap into place immediately).
            if !is_floating {
                WmCtx::X11(ctx_x11.reborrow()).move_resize(
                    win,
                    mon_rect,
                    MoveResizeOptions::animate_to(EMPHASIZED_FRAME_COUNT),
                );
            }

            let _ = ctx_x11
                .x11
                .conn
                .configure_window(x11_win, &ConfigureWindowAux::new().border_width(0));
            // Position and raise the window.
            let _ = ctx_x11.x11.conn.configure_window(
                x11_win,
                &ConfigureWindowAux::new()
                    .x(mon_rect.x)
                    .y(mon_rect.y)
                    .width(mon_rect.w as u32)
                    .height(mon_rect.h as u32),
            );
            let _ = ctx_x11.x11.conn.configure_window(
                x11_win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
            let _ = ctx_x11.x11.conn.flush();
        }

        let mut wmctx = WmCtx::X11(ctx_x11.reborrow());
        sync_monitor_z_order(&mut wmctx, monitor_id);
    } else if !fullscreen && is_fs {
        // ---- Exit fullscreen ------------------------------------------------

        let mut state_atoms = get_atom_props(conn, x11_win, wm_state);
        state_atoms.retain(|&atom| atom != net_wm_fullscreen);
        write_net_wm_state_atoms(conn, x11_win, wm_state, &state_atoms);

        let mut restored_border = 0;

        if let Some(c) = ctx_x11.core.globals_mut().clients.get_mut(&win) {
            c.is_fullscreen = false;
            c.is_floating = c.oldstate != 0;
            c.restore_border_width();
            restored_border = c.border_width.max(0) as u32;
        }

        let _ = ctx_x11.x11.conn.configure_window(
            x11_win,
            &ConfigureWindowAux::new().border_width(restored_border),
        );

        if !is_fake_fs {
            // Snap back to the geometry that was stored before going fullscreen.
            let mut wmctx = WmCtx::X11(ctx_x11.reborrow());
            wmctx.move_resize(win, old_geo, MoveResizeOptions::immediate());
            arrange(&mut wmctx, Some(monitor_id));
        } else {
            let mut wmctx = WmCtx::X11(ctx_x11.reborrow());
            sync_monitor_z_order(&mut wmctx, monitor_id);
        }
    }
}

// ---------------------------------------------------------------------------
// Fake fullscreen toggle
// ---------------------------------------------------------------------------

/// Toggle the "fake fullscreen" mode on the currently selected window.
pub fn toggle_fake_fullscreen_x11(ctx_x11: &mut WmCtxX11<'_>) {
    let Some(win) = ctx_x11.core.selected_client() else {
        return;
    };

    let (is_fullscreen, isfakefullscreen, monitor_id, old_border_width) = ctx_x11
        .core
        .globals()
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
        .unwrap_or((false, false, crate::types::MonitorId(0), 0));

    // Transitioning from fake-fullscreen → real-fullscreen: resize to fill the
    // monitor and raise the window.
    if is_fullscreen && isfakefullscreen {
        let borderpx = ctx_x11.core.globals().cfg.border_width_px;

        let mon_rect = ctx_x11
            .core
            .globals()
            .monitor(monitor_id)
            .map(|m| m.monitor_rect)
            .unwrap_or_default();

        let mut wm_ctx = WmCtx::X11(ctx_x11.reborrow());
        wm_ctx.move_resize(
            win,
            Rect {
                x: mon_rect.x + borderpx,
                y: mon_rect.y + borderpx,
                w: mon_rect.w - 2 * borderpx,
                h: mon_rect.h - 2 * borderpx,
            },
            MoveResizeOptions::immediate(),
        );

        let x11_win: Window = win.into();
        let _ = ctx_x11.x11.conn.configure_window(
            x11_win,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
        let _ = ctx_x11.x11.conn.flush();
    }

    // Restore the border width when leaving fake-fullscreen while still in
    // the fullscreen state (real fullscreen removes the border, so we need to
    // put it back before the layout re-runs).
    if let Some(client) = ctx_x11.core.globals_mut().clients.get_mut(&win) {
        if client.is_fullscreen && !client.isfakefullscreen {
            client.border_width = old_border_width;
        }
        client.isfakefullscreen = !client.isfakefullscreen;
    }
}

pub fn toggle_fake_fullscreen(ctx: &mut WmCtx) {
    match ctx {
        WmCtx::X11(ctx_x11) => toggle_fake_fullscreen_x11(ctx_x11),
        WmCtx::Wayland(_) => {
            if let Some(win) = ctx.selected_client() {
                if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
                    client.isfakefullscreen = !client.isfakefullscreen;
                }
                let selmon_id = ctx.core().globals().selected_monitor_id();
                ctx.core_mut()
                    .globals_mut()
                    .queue_layout_for_monitor_urgent(selmon_id);
            }
        }
    }
}
