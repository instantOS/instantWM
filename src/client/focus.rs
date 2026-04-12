#![allow(clippy::too_many_arguments)]
//! Client focus management and X11 input plumbing.
//!
//! # Responsibilities
//!
//! * [`configure_x11`]  – send a synthetic `ConfigureNotify` to a client so it
//!   knows its current geometry without waiting for a real configure event.
//! * [`send_event_x11`] – send an arbitrary `ClientMessage`, with optional
//!   `WM_PROTOCOLS` existence check.
//! * [`set_focus_x11`]  – give input focus to a client window.
//! * [`unfocus_win_x11`] – remove focus from a client (reset border colour,
//!   optionally redirect focus to the root).
//! * [`grab_buttons_x11`] – (un)grab mouse buttons on a client depending on
//!   whether it is currently focused.

use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::client::constants::WM_HINTS_URGENCY_HINT;
use crate::contexts::CoreCtx;
use crate::types::BarPosition;
use crate::types::WindowId;
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

#[derive(Default)]
pub struct FocusState {
    /// The window currently being animated (0 = none).
    pub anim_client: WindowId,
    /// The previously focused window (0 = none), used by focus-last-client logic.
    pub last_client: WindowId,
}

// ---------------------------------------------------------------------------
// ConfigureNotify
// ---------------------------------------------------------------------------

/// Send a synthetic `ConfigureNotify` event to `win`.
///
/// Some clients (e.g. those that cache their own geometry) need this to
/// learn about position/size changes that did not originate from their own
/// `ConfigureRequest`.  We send it after every [`super::geometry::resize_client`]
/// call.
pub fn configure_x11(core: &mut CoreCtx, x11: &X11BackendRef, win: WindowId) {
    let conn = x11.conn;
    let x11_win: Window = win.into();

    let Some(c) = core.client(win) else {
        return;
    };

    let event = ConfigureNotifyEvent {
        response_type: CONFIGURE_NOTIFY_EVENT,
        sequence: 0,
        event: x11_win,
        window: x11_win,
        above_sibling: 0,
        x: c.geo.x as i16,
        y: c.geo.y as i16,
        width: c.geo.w as u16,
        height: c.geo.h as u16,
        border_width: c.border_width as u16,
        override_redirect: false,
    };

    let _ = conn.send_event(false, x11_win, EventMask::STRUCTURE_NOTIFY, event);
    let _ = conn.flush();
}

// ---------------------------------------------------------------------------
// ClientMessage helper
// ---------------------------------------------------------------------------

/// Send a `ClientMessage` event to `win`.
///
/// When `proto` is one of `WM_TAKE_FOCUS` or `WM_DELETE_WINDOW`, this first
/// checks that the protocol is listed in the window's `WM_PROTOCOLS` property
/// and returns `false` without sending if it is not supported.
///
/// For any other `proto` value the message is sent unconditionally and `true`
/// is returned.
pub fn send_event_x11(
    _core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
    proto: u32,
    mask: u32,
    d0: i64,
    d1: i64,
    d2: i64,
    d3: i64,
    d4: i64,
) -> bool {
    let conn = x11.conn;
    let x11_win: Window = win.into();

    let wmatom_protocols = x11_runtime.wmatom.protocols;
    let wmatom_take_focus = x11_runtime.wmatom.take_focus;
    let wmatom_delete = x11_runtime.wmatom.delete;

    let (exists, message_type) = if proto == wmatom_take_focus || proto == wmatom_delete {
        // Check whether the client advertises support for this protocol.
        let supported = read_wm_protocols(conn, x11_win, x11_runtime.wmatom.protocols)
            .into_iter()
            .any(|p| p == proto);
        (supported, wmatom_protocols)
    } else {
        (true, proto)
    };

    if exists {
        let event = ClientMessageEvent {
            response_type: CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window: x11_win,
            type_: message_type,
            data: ClientMessageData::from([d0 as u32, d1 as u32, d2 as u32, d3 as u32, d4 as u32]),
        };
        let _ = conn.send_event(false, x11_win, EventMask::from(mask), event);
        let _ = conn.flush();
    }

    exists
}

/// Update the border color of `win` based on its current focus and floating state.
pub fn refresh_border_color_x11(
    core: &CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
    focused: bool,
) {
    let scheme = &x11_runtime.borderscheme;
    let Some(c) = core.client(win) else {
        return;
    };

    let pixel = if focused {
        let has_tiling = core.globals().selected_monitor().is_tiling_layout();
        let isfloating = c.is_floating || !has_tiling;
        if isfloating {
            scheme.float_focus.bg.pixel()
        } else {
            scheme.tile_focus.bg.pixel()
        }
    } else {
        scheme.normal.bg.pixel()
    };

    let x11_win: Window = win.into();
    let _ = x11.conn.change_window_attributes(
        x11_win,
        &ChangeWindowAttributesAux::new().border_pixel(Some(pixel)),
    );
}

/// Give input focus to `win`.
///
/// Sets the X input focus (unless the client has `neverfocus` set) and updates
/// `_NET_ACTIVE_WINDOW` on the root.  Also sends `WM_TAKE_FOCUS` so that
/// clients using the "locally active" input model receive focus correctly.
pub fn set_focus_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
) {
    let Some(c) = core.client(win) else {
        return;
    };

    if !c.never_focus {
        let x11_win: Window = win.into();
        let _ = x11
            .conn
            .set_input_focus(InputFocus::POINTER_ROOT, x11_win, CURRENT_TIME);
        let _ = x11.conn.change_property32(
            PropMode::REPLACE,
            x11_runtime.root,
            x11_runtime.netatom.active_window,
            AtomEnum::WINDOW,
            &[x11_win],
        );
    }

    refresh_border_color_x11(core, x11, x11_runtime, win, true);

    grab_buttons_x11(core, x11, x11_runtime, win, true);

    let _ = send_event_x11(
        core,
        x11,
        x11_runtime,
        win,
        x11_runtime.wmatom.take_focus,
        0,
        x11_runtime.wmatom.take_focus as i64,
        CURRENT_TIME as i64,
        0,
        0,
        0,
    );

    let _ = x11.conn.flush();
}

/// Remove focus from `win`.
///
/// Records it in `ctx.focus.last_client`, ungrabs buttons (so that any click on the
/// unfocused window can be intercepted by the WM), and resets the border colour
/// to the normal (unfocused) scheme.
///
/// If `redirect_to_root` is `true`, focus is explicitly returned to the root
/// window and `_NET_ACTIVE_WINDOW` is deleted – this is used when no other
/// client is taking focus.
pub fn unfocus_win_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
    redirect_to_root: bool,
) {
    if win == WindowId::default() {
        return;
    }

    core.focus.last_client = win;
    grab_buttons_x11(core, x11, x11_runtime, win, false);

    // Reset the border to the normal (unfocused) colour.
    refresh_border_color_x11(core, x11, x11_runtime, win, false);

    if redirect_to_root {
        let _ = x11
            .conn
            .set_input_focus(InputFocus::POINTER_ROOT, x11_runtime.root, CURRENT_TIME);
        let _ = x11
            .conn
            .delete_property(x11_runtime.root, x11_runtime.netatom.active_window);
    }

    let _ = x11.conn.flush();
}

// ---------------------------------------------------------------------------
// Button grabs
// ---------------------------------------------------------------------------

/// Grab or ungrab mouse buttons on `win` depending on whether it is focused.
///
/// When `focused` is `false` (i.e. the window is not the current focus), we
/// grab buttons 1 and 3 in all modifier combinations so the WM can intercept
/// clicks and raise/focus the window before passing the event through.
///
/// When `focused` is `true`, plain clicks are released so the client receives
/// them directly, but WM-specific modified `ClientWin` bindings remain grabbed
/// so actions like Super+drag keep working on X11.
pub fn grab_buttons_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
    focused: bool,
) {
    let conn = x11.conn;
    let x11_win: Window = win.into();

    // Always start clean.
    let _ = ungrab_button(conn, ButtonIndex::from(0u8), x11_win, ModMask::ANY);

    let numlockmask = x11_runtime.numlockmask;
    let lock_mask = ModMask::LOCK.bits() as u32;
    let button_mask: u32 = EventMask::BUTTON_PRESS.bits() | EventMask::BUTTON_RELEASE.bits();
    let mut grabs: Vec<(u8, u32)> = Vec::new();

    if !focused {
        grabs.extend([(1, 0), (3, 0)]);
    }

    for button in &core.globals().cfg.buttons {
        if !button.matches(BarPosition::ClientWin) {
            continue;
        }
        if focused && button.mask == 0 {
            continue;
        }

        let grab = (button.button.as_u8(), button.mask);
        if !grabs.contains(&grab) {
            grabs.push(grab);
        }
    }

    // Grab with every combination of NumLock and CapsLock modifiers so the
    // grabs fire regardless of the lock-key state.
    for (button, base_mask) in grabs {
        for &lock_variation in &[0u32, numlockmask, lock_mask, numlockmask | lock_mask] {
            let mods = ModMask::from((base_mask | lock_variation) as u16);
            let _ = conn.grab_button(
                false,
                x11_win,
                button_mask.into(),
                GrabMode::SYNC,
                GrabMode::SYNC,
                0u32,
                0u32,
                button.into(),
                mods,
            );
        }
    }

    let _ = conn.flush();
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Read all atoms listed in the `WM_PROTOCOLS` property of `win`.
/// Returns an empty `Vec` on any error.
fn read_wm_protocols(
    conn: &x11rb::rust_connection::RustConnection,
    win: Window,
    protocols_atom: u32,
) -> Vec<u32> {
    conn.get_property(false, win, protocols_atom, AtomEnum::ATOM, 0, 1024)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|reply| reply.value32().map(|it| it.collect()))
        .unwrap_or_default()
}

/// Un-grab all buttons matching `button` / `modifiers` on `win`.
/// Thin wrapper that makes the call-site intention clear.
fn ungrab_button(
    conn: &x11rb::rust_connection::RustConnection,
    button: ButtonIndex,
    win: Window,
    modifiers: ModMask,
) -> Result<(), x11rb::errors::ConnectionError> {
    conn.ungrab_button(button, win, modifiers)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Urgency hint helper (used by properties.rs)
// ---------------------------------------------------------------------------

/// Clear the `XUrgencyHint` flag in the `WM_HINTS` property of `win`.
///
/// Called after the WM processes an urgency notification on the currently
/// selected window – at that point the urgency is considered "seen".
pub fn clear_urgency_hint_x11(x11: &X11BackendRef, win: WindowId) {
    let conn = x11.conn;
    let x11_win: Window = win.into();

    let Ok(cookie) =
        conn.get_property(false, x11_win, AtomEnum::WM_HINTS, AtomEnum::WM_HINTS, 0, 9)
    else {
        return;
    };

    let Ok(reply) = cookie.reply() else { return };

    let mut data: Vec<u32> = reply.value32().map(|v| v.collect()).unwrap_or_default();

    if data.is_empty() {
        return;
    }

    data[0] &= !WM_HINTS_URGENCY_HINT;

    let _ = conn.change_property32(
        PropMode::REPLACE,
        x11_win,
        AtomEnum::WM_HINTS,
        AtomEnum::WM_HINTS,
        &data,
    );

    let _ = conn.flush();
}
