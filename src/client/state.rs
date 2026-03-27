//! X11 property management for client windows.
//!
//! This module owns everything related to reading and writing X11 properties
//! that describe a client's state.  It is the bridge between the WM's internal
//! bookkeeping and the X server's property store.

use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::client::constants::{
    BROKEN, MWM_DECOR_ALL, MWM_DECOR_BORDER, MWM_DECOR_TITLE, MWM_HINTS_DECORATIONS,
    MWM_HINTS_DECORATIONS_FIELD, MWM_HINTS_FLAGS_FIELD, WM_HINTS_INPUT_HINT, WM_HINTS_URGENCY_HINT,
};
// use crate::client::focus::clear_urgency_hint_x11;
use crate::client::fullscreen::set_fullscreen_x11;
use crate::client::geometry::resize;
use crate::client::rules::WindowProperties;
use crate::client::rules::apply_rules as apply_rules_generic;
use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
use crate::types::{Rect, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

// ---------------------------------------------------------------------------
// WM_STATE
// ---------------------------------------------------------------------------

/// Write the `WM_STATE` property for `win` to the X server.
///
/// `state` should be one of the `WM_STATE_*` constants from
/// [`crate::client::constants`].
pub fn set_client_state(
    _core: &CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
    state: i32,
) {
    let conn = x11.conn;
    let x11_win: Window = win.into();

    // WM_STATE is a pair of CARD32 values: [state, icon_pixmap].
    // ICCCM §4.1.3.1 requires format=32 and a count of 2 items.
    // Using format=8 (the previous code) caused get_property's value32()
    // iterator to return None, making is_hidden() always return false.
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

// ---------------------------------------------------------------------------
// _NET_CLIENT_INFO  (tag mask + monitor number)
// ---------------------------------------------------------------------------

/// Write the `_NET_CLIENT_INFO` property for `win`.
///
/// This is a two-element `CARDINAL` array: `[tags_mask, monitor_num]`.
/// External tools (e.g. `instantmenu`) can read this to know which tags and
/// monitor a window belongs to without querying the WM over IPC.
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

// ---------------------------------------------------------------------------
// _NET_CLIENT_LIST
// ---------------------------------------------------------------------------

/// Rebuild `_NET_CLIENT_LIST` on the root window from scratch.
///
/// The list is rebuilt by iterating over every monitor's client list in
/// focus order.  Clients are appended in the order they appear in the list,
/// which matches the order used by most EWMH-aware taskbars.
pub fn update_client_list(core: &CoreCtx, x11: &X11BackendRef, x11_runtime: &X11RuntimeConfig) {
    let conn = x11.conn;

    // Delete the existing property first so we start with a clean slate.
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

// ---------------------------------------------------------------------------
// Window title
// ---------------------------------------------------------------------------

/// Read the window title and store it in `Client::name`.
///
/// On X11, prefers `_NET_WM_NAME` (UTF-8) over the legacy `WM_NAME` property.
/// On Wayland, reads the title from the XDG toplevel surface data.
/// Falls back to [`BROKEN`] when the title is not available.
pub fn update_title_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
) {
    let name = read_window_title(core, x11, x11_runtime, win);
    if let Some(client) = core.globals_mut().clients.get_mut(&win) {
        client.name = name;
    }
}

/// Read the window title directly from the X server.
///
/// Returns the first non-empty value found among `_NET_WM_NAME` and `WM_NAME`,
/// or [`BROKEN`] if both are absent / unreadable.
fn read_window_title(
    _core: &CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
) -> String {
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

        // Titles are NUL-terminated strings; strip everything from the first NUL.
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

// ---------------------------------------------------------------------------
// Rule matching
// ---------------------------------------------------------------------------

/// Apply the configured window rules to `win`.
///
/// Rules are matched against the window's `WM_CLASS` (class and instance
/// strings) and its title.  Matching rules can set:
///
/// * `isfloating` / layout override (`RuleFloat` variant).
/// * Tag mask (`tags` field).
/// * Target monitor (`monitor` field).
///
/// After rule matching, the final tag mask is clamped to the current tag set.
/// If no rule matches (and `SpecialNext` is `None`), the window inherits its
/// monitor's currently active tags.
pub fn apply_rules(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
) {
    let conn = x11.conn;
    let x11_win: Window = win.into();

    let (class_bytes, instance_bytes) = read_wm_class(conn, x11_win);
    let title = read_window_title(core, x11, x11_runtime, win);

    let props = WindowProperties {
        class: String::from_utf8_lossy(&class_bytes).into_owned(),
        instance: String::from_utf8_lossy(&instance_bytes).into_owned(),
        title,
    };

    apply_rules_generic(core.globals_mut(), win, &props);
}

/// Read the `WM_CLASS` property and return `(class_bytes, instance_bytes)`.
///
/// The property value is `"instance\0class\0"` per ICCCM §4.1.2.5.
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

// ---------------------------------------------------------------------------
// _NET_WM_WINDOW_TYPE / _NET_WM_STATE
// ---------------------------------------------------------------------------

/// Handle `_NET_WM_WINDOW_TYPE` and `_NET_WM_STATE` for a newly managed window.
///
/// * If `_NET_WM_STATE` contains `_NET_WM_STATE_FULLSCREEN`, calls
///   [`set_fullscreen`] to enter fullscreen immediately.
/// * If `_NET_WM_WINDOW_TYPE` is `_NET_WM_WINDOW_TYPE_DIALOG`, marks the
///   client as floating.
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

// ---------------------------------------------------------------------------
// WM_HINTS
// ---------------------------------------------------------------------------

/// Parse `WM_HINTS` for `win` and update `Client::isurgent` / `Client::neverfocus`.
///
/// * If the urgency hint is set on the *currently selected* window, the hint is
///   cleared immediately (the user is already looking at it).
/// * The `neverfocus` flag is derived from the `InputHint` field.
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

/// Set or clear the urgency state on `win`, updating both the internal flag
/// and the `WM_HINTS` property on the X server.
///
/// This function is currently reserved for future EWMH compliance use but is
/// kept here so the property plumbing is in one place.
pub fn set_urgent_x11(core: &mut CoreCtx, x11: &X11BackendRef, win: WindowId, urg: bool) {
    let conn = x11.conn;

    // Update the internal flag first.
    if let Some(client) = core.globals_mut().clients.get_mut(&win) {
        client.is_urgent = urg;
    }

    // Read the current WM_HINTS so we only modify the urgency bit.
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

    // Rebuild the byte array with the updated flags word.
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

// ---------------------------------------------------------------------------
// _MOTIF_WM_HINTS  (border / decoration hints)
// ---------------------------------------------------------------------------

/// Parse `_MOTIF_WM_HINTS` decoration flags and adjust the client's border.
///
/// When the `MWM_HINTS_DECORATIONS` flag is present and no border / title
/// decoration bits are set, the border width is forced to 0.  Otherwise the
/// global `borderpx` value is used.
///
/// This function is a no-op when `decorhints` is disabled in the global config.
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

    // The raw property is an array of 5 × 32-bit values (the MWM hints struct).
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

    // If any decoration bit is set (all, border, or title), keep the normal
    // border; otherwise suppress it entirely.
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

    // Resize to account for the changed border (total size stays the same;
    // the content area grows or shrinks by the border delta).
    {
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
}

// ---------------------------------------------------------------------------
// Internal atom helper
// ---------------------------------------------------------------------------

/// Read a single-atom property from `win` and return its value.
///
/// Returns `None` when the property is absent, empty, or unreadable.
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
