//! X11-specific fullscreen helpers.

use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::backend::x11::properties::{get_atom_props, write_net_wm_state_atoms};
use crate::types::WindowId;
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

/// Add or remove both EWMH maximized atoms for `win`.
pub fn set_maximized_atoms(
    x11: &X11BackendRef<'_>,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
    maximized: bool,
) {
    let x11_win: Window = win.into();
    let wm_state = x11_runtime.netatom.wm_state;
    let atoms = [
        x11_runtime.netatom.wm_maximized_vert,
        x11_runtime.netatom.wm_maximized_horz,
    ];
    let mut state = get_atom_props(x11.conn, x11_win, wm_state);
    if maximized {
        for atom in atoms {
            if !state.contains(&atom) {
                state.push(atom);
            }
        }
    } else {
        state.retain(|atom| !atoms.contains(atom));
    }
    write_net_wm_state_atoms(x11.conn, x11_win, wm_state, &state);
}

/// Remove border from an X11 window (for entering fullscreen).
pub fn remove_border(x11: &X11BackendRef<'_>, win: WindowId) {
    let x11_win: Window = win.into();
    let _ = x11
        .conn
        .configure_window(x11_win, &ConfigureWindowAux::new().border_width(0));
    let _ = x11.conn.flush();
}

/// Restore border width on an X11 window (for exiting fullscreen).
pub fn restore_border(x11: &X11BackendRef<'_>, model: &crate::model::WmModel, win: WindowId) {
    let x11_win: Window = win.into();
    let restored_border = model
        .client(win)
        .map(|c| c.border_width.max(0) as u32)
        .unwrap_or(0);
    let _ = x11.conn.configure_window(
        x11_win,
        &ConfigureWindowAux::new().border_width(restored_border),
    );
}
