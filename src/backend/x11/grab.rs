//! X11 input-grab helpers.
//!
//! Three distinct concepts live here:
//!
//! * **Key grabs** ([`grab_keys_x11`], [`key_press_x11`]) – keyboard event
//!   handling on the X11 root window.
//!
//! * **Button grabs** ([`grab_buttons`]) – passive grabs registered on a
//!   client window so the WM receives button-press events even when that
//!   window is not focused.
//!
//! * **Pointer grabs** ([`grab_pointer`], [`ungrab_ctx`]) – active, modal grabs
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

use crate::contexts::{CoreCtx, WmCtxX11};
use crate::types::{AltCursor, Key, MouseButton, WindowId};
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

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
pub fn grab_pointer(ctx: &WmCtxX11, cursor: AltCursor) -> bool {
    let cursor_index = cursor.to_x11_index();
    let xcursor = ctx
        .x11_runtime
        .cursors
        .get(cursor_index)
        .and_then(|c| c.as_ref())
        .map(|c| c.cursor as u32)
        .unwrap_or(x11rb::NONE);

    grab_pointer_impl(
        ctx.x11.conn,
        ctx.x11_runtime.root,
        xcursor,
        EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
    )
}

/// Like [`grab_pointer`] but additionally listens for `KeyPress` events.
///
/// Used by [`crate::mouse::hover::hover_resize_mouse`] so that pressing
/// Escape can abort the hover-resize wait before the user clicks.
pub fn grab_pointer_with_keys(ctx: &WmCtxX11, cursor: AltCursor) -> bool {
    let cursor_index = cursor.to_x11_index();
    let xcursor = ctx
        .x11_runtime
        .cursors
        .get(cursor_index)
        .and_then(|c| c.as_ref())
        .map(|c| c.cursor as u32)
        .unwrap_or(x11rb::NONE);

    grab_pointer_impl(
        ctx.x11.conn,
        ctx.x11_runtime.root,
        xcursor,
        EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::POINTER_MOTION
            | EventMask::KEY_PRESS,
    )
}

fn grab_pointer_impl<C: Connection>(
    conn: &C,
    root: x11rb::protocol::xproto::Window,
    cursor: u32,
    event_mask: EventMask,
) -> bool {
    conn.grab_pointer(
        false,
        root,
        event_mask,
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

/// Wait for the next X11 event.
///
/// Borrows the connection only for the duration of the call, so the caller
/// can freely mutate `ctx` between events.
pub fn wait_event(ctx: &WmCtxX11) -> Option<x11rb::protocol::Event> {
    ctx.x11.conn.wait_for_event().ok()
}

/// Release an active pointer grab via context.
///
/// Always call this when a drag/resize loop ends, even on early returns,
/// to avoid leaving the pointer permanently grabbed.
#[inline]
pub fn ungrab(ctx: &crate::contexts::WmCtxX11) {
    let _ = ungrab_pointer(ctx.x11.conn, CURRENT_TIME);
    let _ = ctx.x11.conn.flush();
}

/// Generic X11 mouse-drag event loop.
///
/// Handles pointer grabbing, the motion-event loop (with throttling),
/// and final ungrabbing.
///
/// If `with_keys` is true, also captures KeyPress events.
/// The closure `on_event` returns `true` to continue the loop, `false` to break.
pub fn mouse_drag_loop<F>(
    ctx: &mut WmCtxX11<'_>,
    btn: MouseButton,
    cursor: AltCursor,
    with_keys: bool,
    mut on_event: F,
) where
    F: FnMut(&mut WmCtxX11<'_>, &x11rb::protocol::Event) -> bool,
{
    let grabbed = if with_keys {
        grab_pointer_with_keys(ctx, cursor)
    } else {
        grab_pointer(ctx, cursor)
    };

    if !grabbed {
        return;
    }

    loop {
        // Wait for at least one event (blocking).
        let Some(mut event) = wait_event(ctx) else {
            break;
        };

        // If it's a motion event, compress it by eating all subsequent pending
        // motion events in the queue, keeping only the absolute latest.
        // This ensures zero-latency dragging without artificial 16ms FPS caps.
        if let x11rb::protocol::Event::MotionNotify(_) = event {
            while let Ok(Some(next_evt)) = ctx.x11.conn.poll_for_event() {
                if let x11rb::protocol::Event::MotionNotify(_) = next_evt {
                    event = next_evt; // Discard older motion, keep newest.
                } else {
                    // It's a different event (e.g. ButtonRelease). We must put it
                    // back so wait_event/poll_for_event yield it next time!
                    // x11rb doesn't let us un-read events easily, so we process
                    // the compressed motion *now*, then process this next_evt.
                    if !on_event(ctx, &event) {
                        ungrab(ctx);
                        return;
                    }

                    // Now process the non-motion event we peeked.
                    if let x11rb::protocol::Event::ButtonRelease(br) = next_evt
                        && br.detail == btn.as_u8()
                    {
                        ungrab(ctx);
                        return;
                    }
                    if !on_event(ctx, &next_evt) {
                        ungrab(ctx);
                        return;
                    }

                    // We've processed the peeking; continue the main `wait_event` loop.
                    continue;
                }
            }
        }

        let should_continue = match &event {
            x11rb::protocol::Event::ButtonRelease(br) => {
                if br.detail == btn.as_u8() {
                    false
                } else {
                    on_event(ctx, &event)
                }
            }
            _ => on_event(ctx, &event),
        };

        if !should_continue {
            break;
        }
    }

    ungrab(ctx);
}

// ── Passive button grabs ──────────────────────────────────────────────────────

/// Register (or clear) passive button grabs on a client window.
///
/// * When `focused` is **`true`**: all existing grabs are removed.
/// * When `focused` is **`false`**: grabs are installed for buttons 1 and 3
///   with every combination of NumLock and CapsLock modifiers.
pub fn grab_buttons(ctx: &crate::contexts::WmCtxX11, c_win: WindowId, focused: bool) {
    let conn = ctx.x11.conn;
    let x11_win: Window = c_win.into();

    // Always start clean.
    let _ = conn.ungrab_button(0u8.into(), x11_win, ModMask::from(0u16));

    if focused {
        return;
    }

    let numlockmask = ctx.x11_runtime.numlockmask as u16;

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
                x11_win,
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
}

// ── Key grabs ─────────────────────────────────────────────────────────────────

fn grab_keys_for_key<C: Connection>(
    conn: &C,
    root: Window,
    modifiers: &[u16],
    key: &Key,
    keycode: u8,
) {
    for &modif in modifiers {
        let _ = grab_key(
            conn,
            false,
            root,
            ((key.mod_mask as u16) | modif).into(),
            keycode,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        );
    }
}

pub fn grab_keys_x11(
    core: &CoreCtx,
    x11: &crate::backend::x11::X11BackendRef,
    x11_runtime: &crate::backend::x11::X11RuntimeConfig,
) {
    let conn = x11.conn;
    let root = x11_runtime.root;
    let numlockmask = x11_runtime.numlockmask;
    let keys = core.globals().cfg.keys.as_slice();
    let desktop_keybinds = core.globals().cfg.desktop_keybinds.as_slice();
    let modes = &core.globals().cfg.modes;

    let _ = ungrab_key(conn, 0, root, ModMask::ANY);

    let (keycode_min, keycode_max): (u8, u8) = (conn.setup().min_keycode, conn.setup().max_keycode);

    let modifiers: [u16; 4] = [
        0,
        ModMask::LOCK.bits(),
        numlockmask as u16,
        (numlockmask as u16) | ModMask::LOCK.bits(),
    ];

    let mapping = match conn
        .get_keyboard_mapping(keycode_min, keycode_max - keycode_min + 1)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    {
        Some(mapping) => mapping,
        None => return,
    };

    let get_keysym = |keycode: u8| -> u32 {
        let index = (keycode - keycode_min) as usize * mapping.keysyms_per_keycode as usize;
        if index < mapping.keysyms.len() {
            mapping.keysyms[index]
        } else {
            0
        }
    };

    for keycode in keycode_min..=keycode_max {
        let keysym = get_keysym(keycode);
        if keysym == 0 {
            continue;
        }

        for key in keys {
            if keysym == key.keysym {
                grab_keys_for_key(conn, root, &modifiers, key, keycode);
            }
        }

        for mode in modes.values() {
            for key in &mode.keybinds {
                if keysym == key.keysym {
                    grab_keys_for_key(conn, root, &modifiers, key, keycode);
                }
            }
        }

        let selected_window = core.selected_client();
        let current_mode = &core.globals().behavior.current_mode;
        let is_any_mode = !current_mode.is_empty() && current_mode != "default";

        if selected_window.is_none() || is_any_mode {
            for key in desktop_keybinds {
                if keysym == key.keysym {
                    grab_keys_for_key(conn, root, &modifiers, key, keycode);
                }
            }
        }
    }

    let _ = conn.flush();
}

pub fn update_num_lock_mask_x11(
    _core: &mut CoreCtx,
    x11: &crate::backend::x11::X11BackendRef,
    x11_runtime: &mut crate::backend::x11::X11RuntimeConfig,
) {
    let new_numlockmask = {
        let conn = x11.conn;
        let Ok(cookie) = conn.get_modifier_mapping() else {
            return;
        };
        let Ok(reply) = cookie.reply() else {
            return;
        };
        let (keycode_min, keycode_max) = (conn.setup().min_keycode, conn.setup().max_keycode);
        let mapping = match conn
            .get_keyboard_mapping(keycode_min, keycode_max - keycode_min + 1)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
        {
            Some(mapping) => mapping,
            None => return,
        };

        let mut new_numlockmask: u32 = 0;
        for (i, keycode) in reply.keycodes.iter().enumerate() {
            if *keycode >= keycode_min && *keycode <= keycode_max {
                let idx = (*keycode - keycode_min) as usize * mapping.keysyms_per_keycode as usize;
                let keysym = if idx < mapping.keysyms.len() {
                    mapping.keysyms[idx]
                } else {
                    0
                };
                // XK_Num_Lock keysym (X11 keysym 0xff7f) — used to detect
                // which modifier bit corresponds to Num Lock so we can mask
                // it out when matching keybindings.
                if keysym == 0xff7f {
                    let mod_index = i / reply.keycodes_per_modifier() as usize;
                    // X11 supports at most 8 modifier bits (Mod1–Mod5 + Shift/Control/Lock).
                    if mod_index < 8 {
                        new_numlockmask = 1 << mod_index;
                    }
                }
            }
        }

        new_numlockmask
    };

    x11_runtime.numlockmask = new_numlockmask;
}

// ── Keyboard event handlers ───────────────────────────────────────────────────

use crate::keyboard::handle_keysym;
use x11rb::protocol::xproto::{KeyPressEvent, KeyReleaseEvent};

pub fn keycode_to_keysym<C: Connection>(conn: &C, keycode: u8, index: usize) -> u32 {
    if let Ok(cookie) = conn.get_keyboard_mapping(keycode, 1)
        && let Ok(reply) = cookie.reply()
        && index < reply.keysyms_per_keycode as usize
    {
        return reply.keysyms[index];
    }
    0
}

pub fn key_press_x11(ctx: &mut WmCtxX11, e: &KeyPressEvent) {
    let keycode = e.detail;
    let state = e.state;
    let keysym = keycode_to_keysym(ctx.x11.conn, keycode, 0);
    let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
    let _ = handle_keysym(&mut wm_ctx, keysym, state.bits() as u32);
}

pub fn key_release_x11(_ctx: &mut WmCtxX11, _e: &KeyReleaseEvent) {}
