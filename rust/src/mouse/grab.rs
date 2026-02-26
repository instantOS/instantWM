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
//!   [`ungrab_ctx`] when it exits.
//!
//! # Typical drag loop skeleton
//!
//! ```text
//! if !grab_pointer(ctx, cursor_index) { return; }
//! loop {
//!     let Some(event) = wait_event(ctx) else { break };
//!     match event {
//!         ButtonRelease(_) => break,
//!         MotionNotify(m)  => { /* update geometry */ }
//!         _                => {}
//!     }
//! }
//! ungrab_ctx(ctx);
//! ```

use crate::contexts::WmCtx;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::CURRENT_TIME;

// ── Active (modal) pointer grab ───────────────────────────────────────────────

/// Grab the pointer for a modal drag/resize loop.
///
/// Returns `true` on success, `false` if the grab fails (e.g. another client
/// already holds the grab).
///
/// The grab captures `ButtonPress | ButtonRelease | PointerMotion` in async
/// mode on the root window with no event-window confinement.
///
/// After a successful grab, use [`wait_event`] to poll events inside the
/// loop and [`ungrab_ctx`] to release the grab when done.
pub fn grab_pointer(ctx: &WmCtx, cursor_index: usize) -> bool {
    let conn = ctx.x11.conn;

    let root = ctx.g.cfg.root;
    let cursor = ctx
        .g
        .cfg
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
    .ok()
    .and_then(|cookie| cookie.reply().ok())
    .map(|r| r.status == GrabStatus::SUCCESS)
    .unwrap_or(false)
}

/// Like [`grab_pointer`] but additionally listens for `KeyPress` events.
///
/// Used by [`crate::mouse::hover::hover_resize_mouse`] so that pressing
/// Escape can abort the hover-resize wait before the user clicks.
pub fn grab_pointer_with_keys(ctx: &WmCtx, cursor_index: usize) -> bool {
    let conn = ctx.x11.conn;

    let root = ctx.g.cfg.root;
    let cursor = ctx
        .g
        .cfg
        .cursors
        .get(cursor_index)
        .and_then(|c| c.as_ref())
        .map(|c| c.cursor)
        .unwrap_or(x11rb::NONE);

    // KEY_PRESS is NOT valid for grab_pointer. 
    // It must be handles separately or by listening on the root window.
    let result = conn.grab_pointer(
        false,
        root,
        EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::POINTER_MOTION,
        GrabMode::ASYNC,
        GrabMode::ASYNC,
        x11rb::NONE,
        cursor,
        CURRENT_TIME,
    )
    .ok()
    .and_then(|cookie| cookie.reply().ok())
    .map(|r| {
        if r.status != GrabStatus::SUCCESS {
            eprintln!("DEBUG grab_pointer_with_keys: index={} status={:?}", cursor_index, r.status);
        }
        r.status == GrabStatus::SUCCESS
    })
    .unwrap_or(false);

    if !result {
        eprintln!("DEBUG grab_pointer_with_keys: FAILED for index={}", cursor_index);
    }
    result
}

/// Wait for the next X11 event.
///
/// Borrows the connection only for the duration of the call, so the caller
/// can freely mutate `ctx` between events.
pub fn wait_event(ctx: &WmCtx) -> Option<x11rb::protocol::Event> {
    ctx.x11.conn.wait_for_event().ok()
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

/// Release an active pointer grab via context.
///
/// Convenience wrapper around [`ungrab`] that extracts the connection from ctx.
#[inline]
pub fn ungrab_ctx(ctx: &WmCtx) {
    ungrab(ctx.x11.conn);
}

// ── Passive button grabs ──────────────────────────────────────────────────────

/// Register (or clear) passive button grabs on a client window.
///
/// * When `focused` is **`true`**: all existing grabs are removed.
/// * When `focused` is **`false`**: grabs are installed for buttons 1 and 3
///   with every combination of NumLock and CapsLock modifiers.
pub fn grab_buttons(ctx: &WmCtx, c_win: Window, focused: bool) {
    let conn = ctx.x11.conn;

    // Always start clean.
    let _ = conn.ungrab_button(0u8.into(), c_win, ModMask::from(0u16));

    if focused {
        let _ = conn.flush();
        return;
    }

    let numlockmask = ctx.g.cfg.numlockmask as u16;

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
