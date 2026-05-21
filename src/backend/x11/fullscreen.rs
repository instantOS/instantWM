//! X11-specific fullscreen helpers.

use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::backend::x11::properties::{get_atom_props, write_net_wm_state_atoms};
use crate::contexts::{WmCtx, WmCtxX11};
use crate::geometry::MoveResizeOptions;
use crate::globals::Globals;
use crate::types::{ClientMode, Rect, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;

/// Add or remove `_NET_WM_STATE_FULLSCREEN` atom for `win`.
pub fn set_fullscreen_atoms(
    x11: &X11BackendRef<'_>,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
    fullscreen: bool,
) {
    let x11_win: Window = win.into();
    let wm_state = x11_runtime.netatom.wm_state;
    let fullscreen_atom = x11_runtime.netatom.wm_fullscreen;
    let mut state = get_atom_props(x11.conn, x11_win, wm_state);
    if fullscreen {
        if !state.contains(&fullscreen_atom) {
            state.push(fullscreen_atom);
        }
    } else {
        state.retain(|&a| a != fullscreen_atom);
    }
    write_net_wm_state_atoms(x11.conn, x11_win, wm_state, &state);
}

/// Remove border from an X11 window (for entering fullscreen).
pub fn remove_border_x11(x11: &X11BackendRef<'_>, win: WindowId) {
    let x11_win: Window = win.into();
    let _ = x11
        .conn
        .configure_window(x11_win, &ConfigureWindowAux::new().border_width(0));
    let _ = x11.conn.flush();
}

/// Restore border width on an X11 window (for exiting fullscreen).
pub fn restore_border_x11(x11: &X11BackendRef<'_>, globals: &Globals, win: WindowId) {
    let x11_win: Window = win.into();
    let restored_border = globals
        .clients
        .get(&win)
        .map(|c| c.border_width.max(0) as u32)
        .unwrap_or(0);
    let _ = x11.conn.configure_window(
        x11_win,
        &ConfigureWindowAux::new().border_width(restored_border),
    );
}

/// Toggle fake-fullscreen on the selected client (X11 backend).
pub fn toggle_fake_fullscreen_x11(ctx_x11: &mut WmCtxX11<'_>) {
    let Some(win) = ctx_x11.core.globals().selected_win() else {
        return;
    };

    let (mode, monitor_id, old_border_width) = ctx_x11
        .core
        .globals()
        .clients
        .get(&win)
        .map(|c| (c.mode, c.monitor_id, c.old_border_width))
        .unwrap_or((ClientMode::Tiling, crate::types::MonitorId(0), 0));

    // Transitioning from fake-fullscreen → real-fullscreen: resize to fill the
    // monitor and raise the window.
    if mode.is_fake_fullscreen() {
        let borderpx = ctx_x11.core.globals().cfg.window.border_width_px;

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

        wm_ctx.raise_window_visual_only(win);
    }

    // Restore the border width when leaving fake-fullscreen while still in
    // the fullscreen state (real fullscreen removes the border, so we need to
    // put it back before the layout re-runs).
    if let Some(client) = ctx_x11.core.globals_mut().clients.get_mut(&win) {
        match client.mode {
            ClientMode::FakeFullscreen { .. } => client.mode = client.mode.as_fullscreen(),
            ClientMode::TrueFullscreen { .. } => client.mode = client.mode.as_fake_fullscreen(),
            _ => client.mode = client.mode.as_fake_fullscreen(),
        }
        client.border_width = if client.mode.is_true_fullscreen() {
            0
        } else {
            old_border_width
        };
    }
}
