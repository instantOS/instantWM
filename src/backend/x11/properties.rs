//! X11 client property management.
//!
//! This module owns the X11-specific property reads and writes that describe a
//! managed client's state on the X server.

use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::client::constants::{
    BROKEN, MWM_DECOR_ALL, MWM_DECOR_BORDER, MWM_DECOR_TITLE, MWM_HINTS_DECORATIONS,
    MWM_HINTS_DECORATIONS_FIELD, MWM_HINTS_FLAGS_FIELD, WM_HINTS_INPUT_HINT, WM_HINTS_URGENCY_HINT,
};
use crate::client::fullscreen::set_fullscreen_x11;
use crate::client::geometry::resize;
use crate::client::rules::WindowProperties;
use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
use crate::types::{Rect, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

/// Write the `WM_STATE` property for `win` to the X server.
pub fn set_client_state(
    _core: &CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
    state: i32,
) {
    let conn = x11.conn;
    let x11_win: Window = win.into();
    let data: [u32; 2] = [state as u32, 0u32];
    let _ = conn.change_property32(
        PropMode::REPLACE,
        x11_win,
        x11_runtime.wmatom.state,
        x11_runtime.wmatom.state,
        &data,
    );
    let _ = conn.flush();
}

/// Write the `_NET_CLIENT_INFO` property for `win`.
pub fn set_client_tag_prop(
    core: &CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
) {
    let conn = x11.conn;
    let x11_win: Window = win.into();
    let Some(c) = core.globals().clients.get(&win) else {
        return;
    };

    let mon_num = core
        .globals()
        .monitor(c.monitor_id)
        .map(|m| m.num as u32)
        .unwrap_or(0);

    let mut data = [0u8; 8];
    data[..4].copy_from_slice(&c.tags.bits().to_ne_bytes());
    data[4..].copy_from_slice(&mon_num.to_ne_bytes());
    let _ = conn.change_property(
        PropMode::REPLACE,
        x11_win,
        x11_runtime.netatom.client_info,
        AtomEnum::CARDINAL,
        8u8,
        data.len() as u32,
        &data,
    );
    let _ = conn.flush();
}

/// Rebuild `_NET_CLIENT_LIST` on the root window from scratch.
pub fn update_client_list(core: &CoreCtx, x11: &X11BackendRef, x11_runtime: &X11RuntimeConfig) {
    let conn = x11.conn;
    let _ = conn.delete_property(x11_runtime.root, x11_runtime.netatom.client_list);

    for mon in core.globals().monitors_iter_all() {
        for &cur_win in &mon.clients {
            let x11_win: Window = cur_win.into();
            let _ = conn.change_property32(
                PropMode::APPEND,
                x11_runtime.root,
                x11_runtime.netatom.client_list,
                AtomEnum::WINDOW,
                &[x11_win],
            );
        }
    }

    let _ = conn.flush();
}

/// Read the window title and store it in `Client::name`.
pub fn update_title_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
) {
    let name = window_properties_x11(x11, x11_runtime, win).title;
    if let Some(client) = core.globals_mut().clients.get_mut(&win) {
        client.name = name;
    }
}

fn read_window_title(x11: &X11BackendRef, x11_runtime: &X11RuntimeConfig, win: WindowId) -> String {
    let conn = x11.conn;
    let x11_win: Window = win.into();
    let net_wm_name = x11_runtime.netatom.wm_name;

    for atom in [net_wm_name, AtomEnum::WM_NAME.into()] {
        if atom == 0 {
            continue;
        }

        let Ok(cookie) = conn.get_property(false, x11_win, atom, AtomEnum::ANY, 0, 1024) else {
            continue;
        };
        let Ok(reply) = cookie.reply() else { continue };

        if reply.format != 8 || reply.value.is_empty() {
            continue;
        }

        let len = reply
            .value
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(reply.value.len());

        let title = String::from_utf8_lossy(&reply.value[..len]).into_owned();
        if !title.is_empty() {
            return title;
        }
    }

    BROKEN.to_string()
}

/// Read X11 window metadata used by the backend-agnostic rule engine.
pub fn window_properties_x11(
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
) -> WindowProperties {
    let conn = x11.conn;
    let x11_win: Window = win.into();

    let (class_bytes, instance_bytes) = read_wm_class(conn, x11_win);
    let title = read_window_title(x11, x11_runtime, win);

    WindowProperties {
        class: String::from_utf8_lossy(&class_bytes).into_owned(),
        instance: String::from_utf8_lossy(&instance_bytes).into_owned(),
        title,
    }
}

fn read_wm_class(conn: &x11rb::rust_connection::RustConnection, win: Window) -> (Vec<u8>, Vec<u8>) {
    let broken = || BROKEN.as_bytes().to_vec();

    let Ok(cookie) = conn.get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)
    else {
        return (broken(), broken());
    };

    let Ok(reply) = cookie.reply() else {
        return (broken(), broken());
    };

    let data: Vec<u8> = reply.value8().map(|v| v.collect()).unwrap_or_default();
    let parts: Vec<&[u8]> = data.split(|&b| b == 0).filter(|s| !s.is_empty()).collect();

    let instance = parts.first().map(|s| s.to_vec()).unwrap_or_else(broken);
    let class = parts.get(1).map(|s| s.to_vec()).unwrap_or_else(broken);

    (class, instance)
}

/// Handle `_NET_WM_WINDOW_TYPE` and `_NET_WM_STATE` for a newly managed window.
pub fn update_window_type(ctx_x11: &mut WmCtxX11<'_>, win: WindowId) {
    let conn = ctx_x11.x11.conn;
    let x11_win: Window = win.into();
    let state = get_atom_props(conn, x11_win, ctx_x11.x11_runtime.netatom.wm_state);
    let wtype = get_atom_props(conn, x11_win, ctx_x11.x11_runtime.netatom.wm_window_type);

    let atom_fullscreen = ctx_x11.x11_runtime.netatom.wm_fullscreen;
    let atom_dialog = ctx_x11.x11_runtime.netatom.wm_window_type_dialog;

    if state.contains(&atom_fullscreen) {
        set_fullscreen_x11(ctx_x11, win, true);
    }

    if wtype.contains(&atom_dialog)
        && let Some(client) = ctx_x11.core.globals_mut().clients.get_mut(&win)
    {
        client.is_floating = true;
    }
}

/// Parse `WM_HINTS` for `win` and update `Client::isurgent` / `Client::neverfocus`.
pub fn update_wm_hints(ctx: &mut WmCtxX11<'_>, win: WindowId) {
    let conn = ctx.x11.conn;
    let x11_win: Window = win.into();

    let Ok(cookie) =
        conn.get_property(false, x11_win, AtomEnum::WM_HINTS, AtomEnum::WM_HINTS, 0, 9)
    else {
        return;
    };

    let Ok(reply) = cookie.reply() else { return };

    let data: Vec<u32> = reply.value32().map(|v| v.collect()).unwrap_or_default();
    let Some(&flags) = data.first() else { return };

    let input = if flags & WM_HINTS_INPUT_HINT != 0 {
        data.get(1).copied().unwrap_or(0) as i32
    } else {
        0
    };

    let is_urgent = (flags & WM_HINTS_URGENCY_HINT) != 0;

    if let Some(client) = ctx.core.globals_mut().clients.get_mut(&win) {
        client.is_urgent = is_urgent;
        client.never_focus = if flags & WM_HINTS_INPUT_HINT != 0 {
            input == 0
        } else {
            false
        };
    }
}

/// Set or clear the urgency state on `win`, updating both the internal flag and `WM_HINTS`.
pub fn set_urgent_x11(core: &mut CoreCtx, x11: &X11BackendRef, win: WindowId, urg: bool) {
    let conn = x11.conn;

    if let Some(client) = core.globals_mut().clients.get_mut(&win) {
        client.is_urgent = urg;
    }

    let data: Vec<u8> = {
        let x11_win: Window = win.into();
        let Ok(cookie) =
            conn.get_property(false, x11_win, AtomEnum::WM_HINTS, AtomEnum::WM_HINTS, 0, 9)
        else {
            return;
        };
        let Ok(reply) = cookie.reply() else { return };
        reply.value8().map(|v| v.collect()).unwrap_or_default()
    };

    if data.len() < 4 {
        return;
    }

    let flags = u32::from_ne_bytes([data[0], data[1], data[2], data[3]]);
    let new_flags = if urg {
        flags | WM_HINTS_URGENCY_HINT
    } else {
        flags & !WM_HINTS_URGENCY_HINT
    };

    let mut new_data = vec![0u8; data.len().max(36)];
    new_data[..4].copy_from_slice(&new_flags.to_ne_bytes());
    if data.len() > 4 {
        new_data[4..data.len()].copy_from_slice(&data[4..]);
    }

    let x11_win: Window = win.into();
    let _ = conn.change_property(
        PropMode::REPLACE,
        x11_win,
        AtomEnum::WM_HINTS,
        AtomEnum::WM_HINTS,
        8u8,
        new_data.len() as u32,
        &new_data,
    );
    let _ = conn.flush();
}

/// Parse `_MOTIF_WM_HINTS` decoration flags and adjust the client's border.
pub fn update_motif_hints(ctx: &mut WmCtxX11<'_>, win: WindowId) {
    if ctx.core.globals().cfg.decorhints == 0 {
        return;
    }

    let motif_atom = ctx.x11_runtime.motifatom;
    let borderpx = ctx.core.globals().cfg.border_width_px;
    let conn = ctx.x11.conn;
    let x11_win: Window = win.into();

    let Ok(cookie) = conn.get_property(false, x11_win, motif_atom, motif_atom, 0, 5) else {
        return;
    };
    let Ok(reply) = cookie.reply() else { return };

    let data: Vec<u8> = reply.value8().map(|v| v.collect()).unwrap_or_default();
    if data.len() < 20 {
        return;
    }

    let motif: Vec<u32> = data
        .chunks_exact(4)
        .map(|chunk| u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();

    if motif.len() == MWM_HINTS_FLAGS_FIELD
        || (motif[MWM_HINTS_FLAGS_FIELD] & MWM_HINTS_DECORATIONS) == 0
    {
        return;
    }

    let (c_w, c_h, c_x, c_y) = ctx
        .core
        .globals()
        .clients
        .get(&win)
        .map(|c| (c.total_width(), c.total_height(), c.geo.x, c.geo.y))
        .unwrap_or((0, 0, 0, 0));

    let decorations = motif.get(MWM_HINTS_DECORATIONS_FIELD).copied().unwrap_or(0);

    let new_bw = if (decorations & MWM_DECOR_ALL) != 0
        || (decorations & MWM_DECOR_BORDER) != 0
        || (decorations & MWM_DECOR_TITLE) != 0
    {
        borderpx
    } else {
        0
    };

    if let Some(client) = ctx.core.globals_mut().clients.get_mut(&win) {
        client.border_width = new_bw;
        client.old_border_width = new_bw;
    }

    let mut tmp_ctx = WmCtx::X11(ctx.reborrow());
    resize(
        &mut tmp_ctx,
        win,
        &Rect {
            x: c_x,
            y: c_y,
            w: c_w - 2 * new_bw,
            h: c_h - 2 * new_bw,
        },
        false,
    );
}

/// Read a single-atom property from `win` and return its value.
pub fn get_atom_prop(
    conn: &x11rb::rust_connection::RustConnection,
    win: Window,
    atom: u32,
) -> Option<u32> {
    conn.get_property(false, win, atom, AtomEnum::ATOM, 0, 1)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|reply| reply.value32().and_then(|mut it| it.next()))
}

/// Read an atom-list property from `win`.
pub fn get_atom_props(
    conn: &x11rb::rust_connection::RustConnection,
    win: Window,
    atom: u32,
) -> Vec<u32> {
    conn.get_property(false, win, atom, AtomEnum::ATOM, 0, u32::MAX)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|reply| reply.value32().map(|it| it.collect()))
        .unwrap_or_default()
}
