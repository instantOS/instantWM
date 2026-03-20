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
use crate::client::focus::clear_urgency_hint;
use crate::client::fullscreen::set_fullscreen_x11;
use crate::client::geometry::resize;
use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
use crate::types::{MonitorRule, Rect, RuleFloat, SpecialNext, WindowId};
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
    data[..4].copy_from_slice(&c.tags.to_ne_bytes());
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
pub fn apply_rules(core: &mut CoreCtx, x11: &X11BackendRef, win: WindowId) {
    let conn = x11.conn;
    let x11_win: Window = win.into();

    // --- Read WM_CLASS -------------------------------------------------------
    let (class_bytes, instance_bytes) = read_wm_class(conn, x11_win);

    // --- Initialise fields we are about to set -------------------------------
    if let Some(c) = core.globals_mut().clients.get_mut(&win) {
        c.is_floating = false;
        c.tags = 0;
    }

    let special_next = core.globals().behavior.specialnext;
    let rules = core.globals().cfg.rules.clone();
    let tag_mask = core.globals().tags.mask();
    let bar_height = core.globals().cfg.bar_height;

    // --- Handle SpecialNext shortcut or normal rule matching -----------------
    if special_next != SpecialNext::None {
        if let SpecialNext::Float = special_next
            && let Some(c) = core.globals_mut().clients.get_mut(&win)
        {
            c.is_floating = true;
        }
        core.globals_mut().behavior.specialnext = SpecialNext::None;
    } else {
        let client_name = core
            .globals()
            .clients
            .get(&win)
            .map(|c| c.name.clone())
            .unwrap_or_default();

        for rule in &rules {
            if !rule_matches(rule, &client_name, &class_bytes, &instance_bytes) {
                continue;
            }

            // Special case: Onboard (on-screen keyboard) is always sticky.
            if rule.class == Some("Onboard")
                && let Some(c) = core.globals_mut().clients.get_mut(&win)
            {
                c.issticky = true;
            }

            // Look up monitor geometry for FloatFullscreen / Float rules.
            let mon_geo = core
                .globals()
                .clients
                .monitor_id(win)
                .and_then(|mid| core.globals().monitor(mid))
                .map(|m| (m.monitor_rect, m.work_rect, m.showbar));

            if let Some(c) = core.globals_mut().clients.get_mut(&win) {
                apply_float_rule(c, &rule.isfloating, mon_geo, bar_height);
                c.tags |= rule.tags;
            }

            apply_monitor_rule(core, win, rule);
        }
    }

    // --- Clamp tags to the valid tag mask ------------------------------------
    clamp_client_tags(core, win, tag_mask);
}

/// Return `true` when `rule` matches all provided window identifiers.
///
/// Each criterion is optional; an absent criterion always matches.
/// Title is matched against a UTF-8 `String`; class and instance are matched
/// against raw X11 `WM_CLASS` bytes.
fn rule_matches(
    rule: &crate::types::Rule,
    client_name: &str,
    class_bytes: &[u8],
    instance_bytes: &[u8],
) -> bool {
    let title_match = rule
        .title
        .map(|t| bytes_contains(client_name.as_bytes(), t))
        .unwrap_or(true);
    let class_match = rule
        .class
        .map(|c| bytes_contains(class_bytes, c))
        .unwrap_or(true);
    let instance_match = rule
        .instance
        .map(|i| bytes_contains(instance_bytes, i))
        .unwrap_or(true);

    title_match && class_match && instance_match
}

/// Return `true` when `needle` appears as a contiguous subsequence of `haystack`.
#[inline]
fn bytes_contains(haystack: &[u8], needle: &str) -> bool {
    let nb = needle.as_bytes();
    haystack.windows(nb.len()).any(|w| w == nb)
}

/// Apply a `RuleFloat` variant to `client`, optionally adjusting its geometry
/// using the monitor information supplied via `mon_geo`.
///
/// `mon_geo` is `(monitor_rect, work_rect, showbar)` and may be `None` when the
/// client is not yet placed on any monitor (geometry adjustments are skipped).
fn apply_float_rule(
    client: &mut crate::types::client::Client,
    float_rule: &RuleFloat,
    mon_geo: Option<(Rect, Rect, bool)>,
    bar_height: i32,
) {
    let (monitor_rect, work_rect, showbar) = mon_geo.unwrap_or_default();

    match float_rule {
        RuleFloat::FloatCenter => {
            client.is_floating = true;
        }
        RuleFloat::FloatFullscreen => {
            client.is_floating = true;
            client.geo.w = monitor_rect.w;
            client.geo.h = work_rect.h;
            client.geo.x = monitor_rect.x;
            if showbar {
                client.geo.y = monitor_rect.y + bar_height;
            }
        }
        RuleFloat::Scratchpad => {
            client.is_floating = true;
        }
        RuleFloat::Float => {
            client.is_floating = true;
            if showbar {
                client.geo.y = monitor_rect.y + bar_height;
            }
        }
        RuleFloat::Tiled => {
            client.is_floating = false;
        }
    }
}

/// Move `win` to the monitor named in `rule.monitor`, if any.
///
/// The monitor index lookup borrow is fully released before the client map
/// is mutated, satisfying Rust's aliasing rules.
fn apply_monitor_rule(core: &mut CoreCtx, win: WindowId, rule: &crate::types::Rule) {
    let MonitorRule::Index(target_num) = rule.monitor else {
        return;
    };

    let target_mid = core
        .globals()
        .monitors_iter()
        .find(|(_i, m)| m.num == target_num as i32)
        .map(|(i, _)| i);

    if let Some(mid) = target_mid
        && let Some(c) = core.globals_mut().clients.get_mut(&win)
    {
        c.monitor_id = mid;
    }
}

/// Clamp `win`'s tag mask to valid bits and fall back to the monitor's active
/// tags when no rule-assigned tag is currently visible.
fn clamp_client_tags(core: &mut CoreCtx, win: WindowId, tag_mask: u32) {
    let (client_mon_id, client_tags) = core
        .globals()
        .clients
        .get(&win)
        .map(|c| (c.monitor_id, c.tags))
        .unwrap_or((0, 0));

    let Some(mon) = core.globals().monitor(client_mon_id) else {
        return;
    };

    let active_tags = mon.selected_tags();
    let new_tags = if client_tags & tag_mask != 0 {
        client_tags & tag_mask
    } else {
        active_tags
    };

    if let Some(c) = core.globals_mut().clients.get_mut(&win) {
        c.tags = new_tags;
    }
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
    let state = get_atom_prop(conn, x11_win, ctx_x11.x11_runtime.netatom.wm_state);
    let wtype = get_atom_prop(conn, x11_win, ctx_x11.x11_runtime.netatom.wm_window_type);

    let atom_fullscreen = ctx_x11.x11_runtime.netatom.wm_fullscreen;
    let atom_dialog = ctx_x11.x11_runtime.netatom.wm_window_type_dialog;

    if state == Some(atom_fullscreen) {
        set_fullscreen_x11(ctx_x11, win, true);
    }

    if wtype == Some(atom_dialog)
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

    let is_selected = ctx.core.globals().selected_monitor().sel == Some(win);

    // If the window is already focused, clear the urgency flag on the X server
    // so decorations don't keep flashing.
    if is_selected && is_urgent {
        clear_urgency_hint(&ctx.core, &ctx.x11, win);
    }

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
pub fn set_urgent(core: &mut CoreCtx, x11: &X11BackendRef, win: WindowId, urg: bool) {
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
