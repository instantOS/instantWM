//! X11 pointer-grab helpers.
//!
//! Two distinct concepts live here:
//!
//! * **Button grabs** ([`grab_buttons`]) – passive grabs registered on a
//!   client window so the WM receives button-press events even when that
//!   window is not focused.
//!
//! * **Pointer grabs** ([`grab_pointer`], [`ungrab`]) – active, modal grabs
//!   used during interactive move/resize loops.  Every drag loop in
//!   `drag.rs` and `resize.rs` calls [`grab_pointer`] at the start and
//!   [`ungrab`] when it exits.
//!
//! # Typical drag loop skeleton
//!
//! ```text
//! let conn = grab_pointer(cursor_index)?;
//! loop {
//!     match conn.wait_for_event()? {
//!         ButtonRelease(_) => break,
//!         MotionNotify(m)  => { /* update geometry */ }
//!         _                => {}
//!     }
//! }
//! ungrab(conn);
//! ```

use crate::globals::{get_globals, get_x11};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::CURRENT_TIME;

// ── Active (modal) pointer grab ───────────────────────────────────────────────

/// Grab the pointer for a modal drag/resize loop.
///
/// * `cursor_index` is an index into `globals.cursors`:
///   - `0` → normal arrow
///   - `1` → resize (crosshair / corner arrows)
///   - `2` → move (fleur)
///
/// Returns a reference to the connection on success so callers can immediately
/// start their event loop, or `None` if the grab fails (e.g. another client
/// already holds the grab).
///
/// The grab captures `ButtonPress | ButtonRelease | PointerMotion` in async
/// mode on the root window with no event-window confinement.
pub fn grab_pointer(
    cursor_index: usize,
) -> Option<&'static x11rb::rust_connection::RustConnection> {
    let x11 = get_x11();
    let conn = x11.conn.as_ref()?;

    let globals = get_globals();
    let root = globals.root;
    let cursor = globals
        .cursors
        .get(cursor_index)
        .and_then(|c| c.as_ref())
        .map(|c| c.cursor)
        .unwrap_or(x11rb::NONE);

    conn.grab_pointer(
        false,
        root,
        EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
        GrabMode::ASYNC,
        GrabMode::ASYNC,
        x11rb::NONE,
        cursor,
        CURRENT_TIME,
    )
    .ok()?
    .reply()
    .ok()
    .filter(|r| r.status == GrabStatus::SUCCESS)?;

    Some(conn)
}

/// Like [`grab_pointer`] but additionally listens for `KeyPress` events.
///
/// Used by [`crate::mouse::resize::hover_resize_mouse`] so that pressing
/// Escape can abort the hover-resize wait before the user clicks.
pub fn grab_pointer_with_keys(
    cursor_index: usize,
) -> Option<&'static x11rb::rust_connection::RustConnection> {
    let x11 = get_x11();
    let conn = x11.conn.as_ref()?;

    let globals = get_globals();
    let root = globals.root;
    let cursor = globals
        .cursors
        .get(cursor_index)
        .and_then(|c| c.as_ref())
        .map(|c| c.cursor)
        .unwrap_or(x11rb::NONE);

    conn.grab_pointer(
        false,
        root,
        EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::POINTER_MOTION
            | EventMask::KEY_PRESS,
        GrabMode::ASYNC,
        GrabMode::ASYNC,
        x11rb::NONE,
        cursor,
        CURRENT_TIME,
    )
    .ok()?
    .reply()
    .ok()
    .filter(|r| r.status == GrabStatus::SUCCESS)?;

    Some(conn)
}

/// Release an active pointer grab and flush pending requests.
///
/// Always call this when a drag/resize loop ends, even on early returns,
/// to avoid leaving the pointer permanently grabbed.
#[inline]
pub fn ungrab(conn: &x11rb::rust_connection::RustConnection) {
    let _ = ungrab_pointer(conn, CURRENT_TIME);
    let _ = conn.flush();
}

// ── Passive button grabs ──────────────────────────────────────────────────────

/// Register (or clear) passive button grabs on a client window.
///
/// * When `focused` is **`true`**: all existing grabs are removed.  The WM
///   receives pointer events through the normal event mask rather than a grab.
///
/// * When `focused` is **`false`**: grabs are installed for buttons 1 and 3
///   (left- and right-click) with every combination of NumLock and CapsLock
///   modifiers so that accidental lock states do not break focus-follows-click.
///
/// This mirrors the behaviour of the original C `grabbuttons()`.
pub fn grab_buttons(c_win: Window, focused: bool) {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    // Always start clean.
    let _ = conn.ungrab_button(0u8.into(), c_win, ModMask::from(0u16));

    if focused {
        // Focused windows get no passive grabs; events arrive normally.
        let _ = conn.flush();
        return;
    }

    let globals = get_globals();
    let numlockmask = globals.numlockmask as u16;

    // The four modifier combinations that must all be caught:
    //   plain | NumLock | CapsLock | NumLock+CapsLock
    let modifier_variants: [u16; 4] = [
        0,
        numlockmask,
        ModMask::LOCK.bits(),
        numlockmask | ModMask::LOCK.bits(),
    ];

    for &mods in &modifier_variants {
        for &button in &[1u8, 3u8] {
            let _ = conn.grab_button(
                false,
                c_win,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                GrabMode::SYNC,
                GrabMode::SYNC,
                x11rb::NONE,
                x11rb::NONE,
                button.into(),
                ModMask::from(mods),
            );
        }
    }

    let _ = conn.flush();
}
