//! X11-specific keyboard helpers: key grabbing, numlock detection.

use crate::backend::x11::{X11BackendRef, X11RuntimeConfig};
use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
use crate::types::Key;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

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

/// Grab all X11 keybindings for the current config.
pub fn grab_keys_x11(core: &CoreCtx, x11: &X11BackendRef, x11_runtime: &X11RuntimeConfig) {
    let conn = x11.conn;
    let root = x11_runtime.root;
    let numlockmask = x11_runtime.numlockmask;
    let keys = core.globals().cfg.bindings.keys.as_slice();
    let desktop_keybinds = core.globals().cfg.bindings.desktop_keybinds.as_slice();
    let modes = &core.globals().cfg.bindings.modes;

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

        let current_mode = &core.globals().behavior.current_mode;
        let desktop_bindings_enabled =
            crate::keyboard::desktop_bindings_enabled(core.selected_client(), current_mode);

        if desktop_bindings_enabled {
            for key in desktop_keybinds {
                if keysym == key.keysym {
                    grab_keys_for_key(conn, root, &modifiers, key, keycode);
                }
            }
        }
    }

    let _ = conn.flush();
}

/// Update the cached numlock modifier mask from the X11 server.
pub fn update_num_lock_mask_x11(
    _core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
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
                if keysym == 0xff7f {
                    let mod_index = i / reply.keycodes_per_modifier() as usize;
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

/// Convert an X11 keycode to a keysym using the server's keyboard mapping.
pub fn keycode_to_keysym<C: Connection>(conn: &C, keycode: u8, index: usize) -> u32 {
    if let Ok(cookie) = conn.get_keyboard_mapping(keycode, 1)
        && let Ok(reply) = cookie.reply()
        && index < reply.keysyms_per_keycode as usize
    {
        return reply.keysyms[index];
    }
    0
}

/// Handle an X11 `KeyPress` event: convert the keycode to a keysym and dispatch
/// to the backend‑agnostic key handler.
pub fn key_press_x11(ctx: &mut WmCtxX11, e: &KeyPressEvent) {
    let keycode = e.detail;
    let state = e.state;
    let keysym = keycode_to_keysym(ctx.x11.conn, keycode, 0);
    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    let _ = crate::keyboard::handle_keysym(&mut wm_ctx, keysym, state.bits() as u32);
}

/// Handle an X11 `KeyRelease` event (currently a no‑op).
pub fn key_release_x11(_ctx: &mut WmCtxX11, _e: &KeyReleaseEvent) {}
