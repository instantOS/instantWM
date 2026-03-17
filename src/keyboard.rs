use std::rc::Rc;

use crate::backend::x11::X11BackendRef;
use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
use crate::floating::{change_snap, reset_snap, save_floating_geometry, toggle_floating, SnapDir};
use crate::focus::{direction_focus, focus_stack};
use crate::globals::X11RuntimeConfig;

use crate::layouts::arrange;
use crate::overlay::set_overlay_mode;
use crate::scratchpad::unhide_one;
use crate::types::*;
use crate::types::{Direction, StackDirection};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

pub fn keycode_to_keysym<C: Connection>(conn: &C, keycode: u8, index: usize) -> u32 {
    if let Ok(cookie) = conn.get_keyboard_mapping(keycode, 1) {
        if let Ok(reply) = cookie.reply() {
            if index < reply.keysyms_per_keycode as usize {
                return reply.keysyms[index];
            }
        }
    }
    0
}

fn clean_mask(mask: u16, numlockmask: u32) -> u16 {
    let lock_mask = ModMask::LOCK.bits();
    mask & !(numlockmask as u16 | lock_mask)
        & (ModMask::SHIFT.bits()
            | ModMask::CONTROL.bits()
            | ModMask::M1.bits()
            | ModMask::M2.bits()
            | ModMask::M3.bits()
            | ModMask::M4.bits()
            | ModMask::M5.bits())
}

pub fn handle_keysym(ctx: &mut WmCtx, keysym: u32, mod_mask: u32) -> bool {
    let numlockmask = ctx.numlock_mask();
    let cleaned = clean_mask(mod_mask as u16, numlockmask);

    let current_mode = ctx.g().behavior.current_mode.clone();

    let action = if !current_mode.is_empty() && current_mode != "default" {
        // Look ONLY in mode-specific keybindings
        ctx.g()
            .cfg
            .modes
            .get(&current_mode)
            .and_then(|mode| {
                mode.keybinds.iter().find(|key| {
                    keysym == key.keysym && clean_mask(key.mod_mask as u16, numlockmask) == cleaned
                })
            })
            .map(|key| Rc::clone(&key.action))
    } else {
        // Normal mode
        ctx.g()
            .cfg
            .keys
            .iter()
            .find(|key| {
                keysym == key.keysym && clean_mask(key.mod_mask as u16, numlockmask) == cleaned
            })
            .or_else(|| {
                if ctx.selected_client().is_none() {
                    ctx.g().cfg.desktop_keybinds.iter().find(|key| {
                        keysym == key.keysym
                            && clean_mask(key.mod_mask as u16, numlockmask) == cleaned
                    })
                } else {
                    None
                }
            })
            .map(|key| Rc::clone(&key.action))
    };

    if let Some(action) = action {
        action(ctx);
        true
    } else {
        false
    }
}

pub fn key_press_x11(ctx: &mut WmCtxX11, e: &KeyPressEvent) {
    let keycode = e.detail;
    let state = e.state;
    let keysym = keycode_to_keysym(ctx.x11.conn, keycode, 0);
    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    let _ = handle_keysym(&mut wm_ctx, keysym, state.bits() as u32);
}

pub fn key_release_x11(_ctx: &mut WmCtxX11, _e: &KeyReleaseEvent) {}

pub fn grab_keys_x11(core: &CoreCtx, x11: &X11BackendRef, x11_runtime: &X11RuntimeConfig) {
    let conn = x11.conn;
    let root = x11_runtime.root;
    let numlockmask = x11_runtime.numlockmask;
    let keys = core.g.cfg.keys.as_slice();
    let desktop_keybinds = core.g.cfg.desktop_keybinds.as_slice();
    let modes = &core.g.cfg.modes;
    let free_alt_tab = true;

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
                for &modif in &modifiers {
                    if free_alt_tab && key.mod_mask == ModMask::M1.bits() as u32 {
                        continue;
                    }
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
        }

        for mode in modes.values() {
            for key in &mode.keybinds {
                if keysym == key.keysym {
                    for &modif in &modifiers {
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
            }
        }

        let selected_window = core.selected_client();
        if selected_window.is_none() {
            for key in desktop_keybinds {
                if keysym == key.keysym {
                    for &modif in &modifiers {
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
            }
        }
    }

    let _ = conn.flush();
}

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

pub fn up_press(ctx: &mut WmCtx) {
    let (selected_window, overlay_win, is_floating) = {
        let mon = ctx.g().selected_monitor();
        let sel = mon.sel;
        let overlay = mon.overlay;
        let is_floating = sel.map_or(false, |w| ctx.g().clients.is_floating(w));
        (sel, overlay, is_floating)
    };

    if selected_window.is_none() {
        return;
    }

    if selected_window == overlay_win {
        set_overlay_mode(ctx, OverlayMode::Top);
        return;
    }

    if is_floating {
        toggle_floating(ctx);
        return;
    }

    if let Some(win) = selected_window {
        crate::client::hide(ctx, win);
    }
}

pub fn down_press(ctx: &mut WmCtx) {
    if unhide_one(ctx) {
        return;
    }

    let (selected_window, overlay_win, snap_status, is_floating) = {
        let mon = ctx.g().selected_monitor();
        let sel = mon.sel;
        let overlay = mon.overlay;
        let (snap_status, is_floating) = sel
            .and_then(|w| {
                ctx.g()
                    .clients
                    .get(&w)
                    .map(|c| (c.snap_status, c.is_floating))
            })
            .unwrap_or((SnapPosition::None, false));
        (sel, overlay, snap_status, is_floating)
    };

    if selected_window.is_none() {
        return;
    }

    if snap_status != SnapPosition::None {
        if let Some(win) = selected_window {
            reset_snap(ctx, win);
        }
        return;
    }

    if selected_window == overlay_win {
        set_overlay_mode(ctx, OverlayMode::Bottom);
        return;
    }

    if !is_floating {
        toggle_floating(ctx);
    }
}

pub fn up_key(ctx: &mut WmCtx, direction: StackDirection) {
    let is_overview = !ctx.g().selected_monitor().is_tiling_layout();

    if is_overview {
        direction_focus(ctx, Direction::Up);
        return;
    }

    let has_tiling = ctx.g().selected_monitor().is_tiling_layout();

    if !has_tiling {
        if let Some(win) = ctx.selected_client() {
            if let WmCtx::X11(ref x11_ctx) = ctx {
                crate::client::refresh_border_color_x11(
                    &x11_ctx.core,
                    &x11_ctx.x11,
                    x11_ctx.x11_runtime,
                    win,
                    false,
                );
            }
            change_snap(ctx, win, SnapDir::Up);
        }
        return;
    }

    focus_stack(ctx, direction);
}

pub fn down_key(ctx: &mut WmCtx, direction: StackDirection) {
    let is_overview = !ctx.g().selected_monitor().is_tiling_layout();

    if is_overview {
        direction_focus(ctx, Direction::Down);
        return;
    }

    let has_tiling = ctx.g().selected_monitor().is_tiling_layout();

    if !has_tiling {
        if let Some(win) = ctx.selected_client() {
            change_snap(ctx, win, SnapDir::Down);
        }
        return;
    }

    focus_stack(ctx, direction);
}

pub fn space_toggle(ctx: &mut WmCtx) {
    let has_tiling = ctx.g().selected_monitor().is_tiling_layout();

    if !has_tiling {
        let Some(win) = ctx.selected_client() else {
            return;
        };

        let snap_status = {
            ctx.g()
                .clients
                .get(&win)
                .map(|c| c.snap_status)
                .unwrap_or(SnapPosition::None)
        };

        if snap_status != SnapPosition::None {
            reset_snap(ctx, win);
        } else {
            let border_width = ctx.g().cfg.border_width_px;
            ctx.set_border(win, border_width);

            save_floating_geometry(ctx, win);

            if let Some(client) = ctx.g_mut().clients.get_mut(&win) {
                client.snap_status = SnapPosition::Maximized;
            }

            let selmon_id = ctx.g().selected_monitor_id();
            arrange(ctx, Some(selmon_id));
        }
    } else {
        toggle_floating(ctx);
    }
}
