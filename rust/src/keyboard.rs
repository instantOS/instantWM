use std::rc::Rc;

use crate::contexts::WmCtx;
use crate::floating::{change_snap, reset_snap, save_floating_win, toggle_floating, SnapDir};
use crate::focus::{direction_focus, focus_stack};
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
    let numlockmask = ctx.g.cfg.numlockmask;
    let cleaned = clean_mask(mod_mask as u16, numlockmask);

    let action = ctx
        .g
        .cfg
        .keys
        .iter()
        .find(|key| {
            keysym == key.keysym && clean_mask(key.mod_mask as u16, numlockmask) == cleaned
        })
        .or_else(|| {
            if ctx.g.selected_win().is_none() {
                ctx.g.cfg.desktop_keybinds.iter().find(|key| {
                    keysym == key.keysym
                        && clean_mask(key.mod_mask as u16, numlockmask) == cleaned
                })
            } else {
                None
            }
        })
        .map(|key| Rc::clone(&key.action));

    if let Some(action) = action {
        action(ctx);
        true
    } else {
        false
    }
}

pub fn key_press(ctx: &mut WmCtx, e: &KeyPressEvent) {
    let keycode = e.detail;
    let state = e.state;

    let keysym = {
        let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
            return;
        };
        keycode_to_keysym(conn, keycode, 0)
    };

    let _ = handle_keysym(ctx, keysym, state.bits() as u32);
}

pub fn key_release(_ctx: &mut WmCtx, _e: &KeyReleaseEvent) {}

pub fn grab_keys(ctx: &WmCtx) {
    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return;
    };
    let root = ctx.g.cfg.root;
    let numlockmask = ctx.g.cfg.numlockmask;
    let keys = ctx.g.cfg.keys.as_slice();
    let desktop_keybinds = ctx.g.cfg.desktop_keybinds.as_slice();
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
        for key in keys {
            let keysym = get_keysym(keycode);
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

        let sel_win = ctx.g.selected_win();
        if sel_win.is_none() {
            for key in desktop_keybinds {
                let keysym = get_keysym(keycode);
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

pub fn update_num_lock_mask(ctx: &mut WmCtx) {
    let new_numlockmask = {
        let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
            return;
        };
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

    ctx.g.cfg.numlockmask = new_numlockmask;
}

pub fn up_press(ctx: &mut WmCtx) {
    let (sel_win, overlay_win, is_floating) = {
        let Some(mon) = ctx.g.selmon() else {
            return;
        };
        let sel = mon.sel;
        let overlay = mon.overlay;
        let is_floating = sel
            .and_then(|w| ctx.g.clients.get(&w).map(|c| c.isfloating))
            .unwrap_or(false);
        (sel, overlay, is_floating)
    };

    if sel_win.is_none() {
        return;
    }

    if sel_win == overlay_win {
        set_overlay_mode(ctx, OverlayMode::Top);
        return;
    }

    if is_floating {
        toggle_floating(ctx);
        return;
    }

    if let Some(win) = sel_win {
        crate::client::hide(ctx, win);
    }
}

pub fn down_press(ctx: &mut WmCtx) {
    if unhide_one(ctx) {
        return;
    }

    let (sel_win, overlay_win, snapstatus, is_floating) = {
        let Some(mon) = ctx.g.selmon() else {
            return;
        };
        let sel = mon.sel;
        let overlay = mon.overlay;
        let (snapstatus, is_floating) = sel
            .and_then(|w| ctx.g.clients.get(&w).map(|c| (c.snapstatus, c.isfloating)))
            .unwrap_or((SnapPosition::None, false));
        (sel, overlay, snapstatus, is_floating)
    };

    if sel_win.is_none() {
        return;
    }

    if snapstatus != SnapPosition::None {
        if let Some(win) = sel_win {
            reset_snap(ctx, win);
        }
        return;
    }

    if sel_win == overlay_win {
        set_overlay_mode(ctx, OverlayMode::Bottom);
        return;
    }

    if !is_floating {
        toggle_floating(ctx);
    }
}

pub fn up_key(ctx: &mut WmCtx, direction: StackDirection) {
    let is_overview = ctx
        .g
        .selmon()
        .map(|mon| mon.is_tiling_layout())
        .unwrap_or(false);

    if is_overview {
        direction_focus(ctx, Direction::Up);
        return;
    }

    let has_tiling = ctx
        .g
        .selmon()
        .map(|mon| mon.is_tiling_layout())
        .unwrap_or(true);

    if !has_tiling {
        if let Some(win) = ctx.g.selected_win() {
            let border_pixel = ctx.g.cfg.borderscheme.as_ref().map(|s| s.normal.bg.pixel());
            if let (Some(conn), Some(pixel)) = (ctx.x11_conn().map(|x11| x11.conn), border_pixel) {
                let x11_win: Window = win.into();
                let _ = change_window_attributes(
                    conn,
                    x11_win,
                    &ChangeWindowAttributesAux::new().border_pixel(Some(pixel)),
                );
                let _ = conn.flush();
            }
            change_snap(ctx, win, SnapDir::Up);
        }
        return;
    }

    focus_stack(ctx, direction);
}

pub fn down_key(ctx: &mut WmCtx, direction: StackDirection) {
    let is_overview = ctx
        .g
        .selmon()
        .map(|mon| mon.is_tiling_layout())
        .unwrap_or(false);

    if is_overview {
        direction_focus(ctx, Direction::Down);
        return;
    }

    let has_tiling = ctx
        .g
        .selmon()
        .map(|mon| mon.is_tiling_layout())
        .unwrap_or(true);

    if !has_tiling {
        if let Some(win) = ctx.g.selected_win() {
            change_snap(ctx, win, SnapDir::Down);
        }
        return;
    }

    focus_stack(ctx, direction);
}

pub fn space_toggle(ctx: &mut WmCtx) {
    let has_tiling = ctx
        .g
        .selmon()
        .map(|mon| mon.is_tiling_layout())
        .unwrap_or(true);

    if !has_tiling {
        let Some(win) = ctx.g.selected_win() else {
            return;
        };

        let snapstatus = {
            ctx.g
                .clients
                .get(&win)
                .map(|c| c.snapstatus)
                .unwrap_or(SnapPosition::None)
        };

        if snapstatus != SnapPosition::None {
            reset_snap(ctx, win);
        } else {
            let border_pixel = ctx.g.cfg.borderscheme.as_ref().map(|s| s.normal.bg.pixel());
            if let (Some(conn), Some(pixel)) = (ctx.x11_conn().map(|x11| x11.conn), border_pixel) {
                let x11_win: Window = win.into();
                let _ = change_window_attributes(
                    conn,
                    x11_win,
                    &ChangeWindowAttributesAux::new().border_pixel(Some(pixel)),
                );
                let _ = conn.flush();
            }

            save_floating_win(ctx, win);

            if let Some(client) = ctx.g.clients.get_mut(&win) {
                client.snapstatus = SnapPosition::Maximized;
            }

            arrange(ctx, Some(ctx.g.selmon_id()));
        }
    } else {
        toggle_floating(ctx);
    }
}
