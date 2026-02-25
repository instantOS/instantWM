//! Client focus management and X11 input plumbing.
//!
//! # Responsibilities
//!
//! * [`configure`]     – send a synthetic `ConfigureNotify` to a client so it
//!                       knows its current geometry without waiting for a real
//!                       configure event.
//! * [`send_event`]    – send an arbitrary `ClientMessage`, with optional
//!                       `WM_PROTOCOLS` existence check.
//! * [`set_focus`]     – give input focus to a client window.
//! * [`unfocus_win`]   – remove focus from a client (reset border colour,
//!                       optionally redirect focus to the root).
//! * [`grab_buttons`]  – (un)grab mouse buttons on a client depending on
//!                       whether it is currently focused.

use crate::client::constants::WM_HINTS_URGENCY_HINT;
use crate::contexts::WmCtx;
use std::sync::atomic::{AtomicU32, Ordering};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;
use x11rb::CURRENT_TIME;

// ---------------------------------------------------------------------------
// Shared atomics (also used by lifecycle.rs / kill.rs)
// ---------------------------------------------------------------------------

/// The window currently being animated (0 = none).
pub static ANIM_CLIENT: AtomicU32 = AtomicU32::new(0);

/// The previously focused window (0 = none), used by focus-last-client logic.
pub static LAST_CLIENT: AtomicU32 = AtomicU32::new(0);

// ---------------------------------------------------------------------------
// ConfigureNotify
// ---------------------------------------------------------------------------

/// Send a synthetic `ConfigureNotify` event to `win`.
///
/// Some clients (e.g. those that cache their own geometry) need this to
/// learn about position/size changes that did not originate from their own
/// `ConfigureRequest`.  We send it after every [`super::geometry::resize_client`]
/// call.
pub fn configure(ctx: &mut WmCtx, win: Window) {
    let conn = ctx.x11.conn;

    let Some(c) = ctx.g.clients.get(&win) else {
        return;
    };

    let event = ConfigureNotifyEvent {
        response_type: CONFIGURE_NOTIFY_EVENT,
        sequence: 0,
        event: win,
        window: win,
        above_sibling: 0,
        x: c.geo.x as i16,
        y: c.geo.y as i16,
        width: c.geo.w as u16,
        height: c.geo.h as u16,
        border_width: c.border_width as u16,
        override_redirect: false,
    };

    let _ = conn.send_event(false, win, EventMask::STRUCTURE_NOTIFY, event);
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
pub fn send_event(
    ctx: &mut WmCtx,
    win: Window,
    proto: u32,
    mask: u32,
    d0: i64,
    d1: i64,
    d2: i64,
    d3: i64,
    d4: i64,
) -> bool {
    let conn = ctx.x11.conn;

    let wmatom_protocols = ctx.g.cfg.wmatom.protocols;
    let wmatom_take_focus = ctx.g.cfg.wmatom.take_focus;
    let wmatom_delete = ctx.g.cfg.wmatom.delete;

    let (exists, message_type) = if proto == wmatom_take_focus || proto == wmatom_delete {
        // Check whether the client advertises support for this protocol.
        let supported = read_wm_protocols(conn, win, ctx.g.cfg.wmatom.protocols)
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
            window: win,
            type_: message_type,
            data: ClientMessageData::from([d0 as u32, d1 as u32, d2 as u32, d3 as u32, d4 as u32]),
        };
        let _ = conn.send_event(false, win, EventMask::from(mask), event);
        let _ = conn.flush();
    }

    exists
}

// ---------------------------------------------------------------------------
// Focus
// ---------------------------------------------------------------------------

/// Give input focus to `win`.
///
/// Sets the X input focus (unless the client has `neverfocus` set) and updates
/// `_NET_ACTIVE_WINDOW` on the root.  Also sends `WM_TAKE_FOCUS` so that
/// clients using the "locally active" input model receive focus correctly.
pub fn set_focus(ctx: &mut WmCtx, win: Window) {
    let conn = ctx.x11.conn;

    let Some(c) = ctx.g.clients.get(&win) else {
        return;
    };

    if !c.neverfocus {
        let _ = conn.set_input_focus(InputFocus::POINTER_ROOT, win, CURRENT_TIME);
        let _ = conn.change_property32(
            PropMode::REPLACE,
            ctx.g.cfg.root,
            ctx.g.cfg.netatom.active_window,
            AtomEnum::WINDOW,
            &[win],
        );
    }

    send_event(
        ctx,
        win,
        ctx.g.cfg.wmatom.take_focus,
        0,
        ctx.g.cfg.wmatom.take_focus as i64,
        CURRENT_TIME as i64,
        0,
        0,
        0,
    );

    let _ = conn.flush();
}

/// Remove focus from `win`.
///
/// Records it in [`LAST_CLIENT`], ungrabs buttons (so that any click on the
/// unfocused window can be intercepted by the WM), and resets the border colour
/// to the normal (unfocused) scheme.
///
/// If `redirect_to_root` is `true`, focus is explicitly returned to the root
/// window and `_NET_ACTIVE_WINDOW` is deleted – this is used when no other
/// client is taking focus.
pub fn unfocus_win(ctx: &mut WmCtx, win: Window, redirect_to_root: bool) {
    if win == 0 {
        return;
    }

    LAST_CLIENT.store(win, Ordering::Relaxed);
    grab_buttons(ctx, win, false);

    let conn = ctx.x11.conn;

    // Reset the border to the normal (unfocused) colour.
    if let Some(ref scheme) = ctx.g.cfg.borderscheme {
        let pixel = scheme.normal.bg.pixel();
        let _ = conn
            .change_window_attributes(win, &ChangeWindowAttributesAux::new().border_pixel(pixel));
    }

    if redirect_to_root {
        let _ = conn.set_input_focus(InputFocus::POINTER_ROOT, ctx.g.cfg.root, CURRENT_TIME);
        let _ = conn.delete_property(ctx.g.cfg.root, ctx.g.cfg.netatom.active_window);
    }

    let _ = conn.flush();
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
/// When `focused` is `true`, all button grabs are released so the client
/// receives button events directly.
pub fn grab_buttons(ctx: &mut WmCtx, win: Window, focused: bool) {
    let conn = ctx.x11.conn;

    // Always start clean.
    let _ = ungrab_button(conn, ButtonIndex::from(0u8), win, ModMask::from(0u16));

    if !focused {
        let numlockmask = ctx.g.cfg.numlockmask;
        let lock_mask = ModMask::LOCK.bits() as u32;
        let button_mask: u32 = EventMask::BUTTON_PRESS.bits() | EventMask::BUTTON_RELEASE.bits();

        // Grab with every combination of NumLock and CapsLock modifiers so the
        // grabs fire regardless of the lock-key state.
        for &modifiers in &[0u32, numlockmask, lock_mask, numlockmask | lock_mask] {
            let mods = ModMask::from(modifiers as u16);

            // Button 1 (left click) – raise/focus the window.
            let _ = conn.grab_button(
                false,
                win,
                button_mask.into(),
                GrabMode::SYNC,
                GrabMode::SYNC,
                0u32,
                0u32,
                1u8.into(),
                mods,
            );

            // Button 3 (right click) – raise/focus the window.
            let _ = conn.grab_button(
                false,
                win,
                button_mask.into(),
                GrabMode::SYNC,
                GrabMode::SYNC,
                0u32,
                0u32,
                3u8.into(),
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
pub fn clear_urgency_hint(ctx: &mut WmCtx, win: Window) {
    let conn = ctx.x11.conn;

    let Ok(cookie) = conn.get_property(false, win, AtomEnum::WM_HINTS, AtomEnum::WM_HINTS, 0, 9)
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
        win,
        AtomEnum::WM_HINTS,
        AtomEnum::WM_HINTS,
        &data,
    );

    let _ = conn.flush();
}
