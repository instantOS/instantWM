use std::rc::Rc;

use crate::contexts::{CoreCtx, WmCtx, WmCtxX11, X11Ctx};
use crate::floating::{change_snap, reset_snap, save_floating_win, toggle_floating, SnapDir};
use crate::focus::{direction_focus_x11, focus_stack_x11};
use crate::layouts::arrange;
use crate::overlay::set_overlay_mode;
use crate::scratchpad::unhide_one;
use crate::types::*;
use crate::types::{Direction, StackDirection};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

fn with_wm_ctx_x11<T>(core: &mut CoreCtx, x11: &X11Ctx, f: impl FnOnce(&mut WmCtx) -> T) -> T {
    let mut ctx = WmCtx::X11(WmCtxX11 {
        core: core.reborrow(),
        backend: crate::backend::BackendRef::from_x11(x11.conn, x11.screen_num),
        x11: X11Ctx {
            conn: x11.conn,
            screen_num: x11.screen_num,
        },
    });
    f(&mut ctx)
}

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
    let numlockmask = ctx.g().x11.numlockmask;
    let cleaned = clean_mask(mod_mask as u16, numlockmask);

    let action = ctx
        .g()
        .cfg
        .keys
        .iter()
        .find(|key| keysym == key.keysym && clean_mask(key.mod_mask as u16, numlockmask) == cleaned)
        .or_else(|| {
            if ctx.selected_client().is_none() {
                ctx.g().cfg.desktop_keybinds.iter().find(|key| {
                    keysym == key.keysym && clean_mask(key.mod_mask as u16, numlockmask) == cleaned
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

pub fn key_press_x11(ctx: &mut WmCtx, e: &KeyPressEvent) {
    let keycode = e.detail;
    let state = e.state;

    let keysym = match ctx {
        WmCtx::X11(ref x11_ctx) => keycode_to_keysym(x11_ctx.x11.conn, keycode, 0),
        _ => return,
    };

    let _ = handle_keysym(ctx, keysym, state.bits() as u32);
}

pub fn key_release_x11(_ctx: &mut WmCtx, _e: &KeyReleaseEvent) {}

pub fn grab_keys_x11(core: &CoreCtx, x11: &X11Ctx) {
    let conn = x11.conn;
    let root = core.g.x11.root;
    let numlockmask = core.g.x11.numlockmask;
    let keys = core.g.cfg.keys.as_slice();
    let desktop_keybinds = core.g.cfg.desktop_keybinds.as_slice();
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

        let selected_window = core.selected_client();
        if selected_window.is_none() {
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

pub fn update_num_lock_mask_x11(core: &mut CoreCtx, x11: &X11Ctx) {
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

    core.g.x11.numlockmask = new_numlockmask;
}

pub fn up_press_x11(core: &mut CoreCtx, x11: &X11Ctx) {
    let (selected_window, overlay_win, is_floating) = {
        let mon = core.g.selected_monitor();
        let sel = mon.sel;
        let overlay = mon.overlay;
        let is_floating = sel
            .and_then(|w| core.g.clients.get(&w).map(|c| c.isfloating))
            .unwrap_or(false);
        (sel, overlay, is_floating)
    };

    if selected_window.is_none() {
        return;
    }

    if selected_window == overlay_win {
        with_wm_ctx_x11(core, x11, |ctx| set_overlay_mode(ctx, OverlayMode::Top));
        return;
    }

    if is_floating {
        with_wm_ctx_x11(core, x11, |ctx| toggle_floating(ctx));
        return;
    }

    if let Some(win) = selected_window {
        crate::client::hide_x11(core, x11, win);
    }
}

pub fn down_press_x11(core: &mut CoreCtx, x11: &X11Ctx) {
    if with_wm_ctx_x11(core, x11, |ctx| unhide_one(ctx)) {
        return;
    }

    let (selected_window, overlay_win, snap_status, is_floating) = {
        let mon = core.g.selected_monitor();
        let sel = mon.sel;
        let overlay = mon.overlay;
        let (snap_status, is_floating) = sel
            .and_then(|w| {
                core.g
                    .clients
                    .get(&w)
                    .map(|c| (c.snap_status, c.isfloating))
            })
            .unwrap_or((SnapPosition::None, false));
        (sel, overlay, snap_status, is_floating)
    };

    if selected_window.is_none() {
        return;
    }

    if snap_status != SnapPosition::None {
        if let Some(win) = selected_window {
            let mut ctx_x11 = WmCtxX11 {
                core: core.reborrow(),
                backend: crate::backend::BackendRef::from_x11(x11.conn, x11.screen_num),
                x11: X11Ctx {
                    conn: x11.conn,
                    screen_num: x11.screen_num,
                },
            };
            reset_snap(&mut ctx_x11, win);
        }
        return;
    }

    if selected_window == overlay_win {
        with_wm_ctx_x11(core, x11, |ctx| set_overlay_mode(ctx, OverlayMode::Bottom));
        return;
    }

    if !is_floating {
        with_wm_ctx_x11(core, x11, |ctx| toggle_floating(ctx));
    }
}

pub fn up_key_x11(core: &mut CoreCtx, x11: &X11Ctx, direction: StackDirection) {
    let is_overview = !core.g.selected_monitor().is_tiling_layout();

    if is_overview {
        direction_focus_x11(core, x11, Direction::Up);
        return;
    }

    let has_tiling = core.g.selected_monitor().is_tiling_layout();

    if !has_tiling {
        if let Some(win) = core.selected_client() {
            let border_pixel = core
                .g
                .cfg
                .borderscheme
                .as_ref()
                .map(|s| s.normal.bg.pixel());
            if let Some(pixel) = border_pixel {
                let x11_win: Window = win.into();
                let _ = change_window_attributes(
                    x11.conn,
                    x11_win,
                    &ChangeWindowAttributesAux::new().border_pixel(Some(pixel)),
                );
                let _ = x11.conn.flush();
            }
            with_wm_ctx_x11(core, x11, |ctx| change_snap(ctx, win, SnapDir::Up));
        }
        return;
    }

    focus_stack_x11(core, x11, direction);
}

pub fn down_key_x11(core: &mut CoreCtx, x11: &X11Ctx, direction: StackDirection) {
    let is_overview = core.g.selected_monitor().is_tiling_layout();

    if is_overview {
        direction_focus_x11(core, x11, Direction::Down);
        return;
    }

    let has_tiling = core.g.selected_monitor().is_tiling_layout();

    if !has_tiling {
        if let Some(win) = core.selected_client() {
            with_wm_ctx_x11(core, x11, |ctx| change_snap(ctx, win, SnapDir::Down));
        }
        return;
    }

    focus_stack_x11(core, x11, direction);
}

pub fn space_toggle_x11(core: &mut CoreCtx, x11: &X11Ctx) {
    let has_tiling = core.g.selected_monitor().is_tiling_layout();

    if !has_tiling {
        let Some(win) = core.selected_client() else {
            return;
        };

        let snap_status = {
            core.g
                .clients
                .get(&win)
                .map(|c| c.snap_status)
                .unwrap_or(SnapPosition::None)
        };

        if snap_status != SnapPosition::None {
            let mut ctx_x11 = WmCtxX11 {
                core: core.reborrow(),
                backend: crate::backend::BackendRef::from_x11(x11.conn, x11.screen_num),
                x11: X11Ctx {
                    conn: x11.conn,
                    screen_num: x11.screen_num,
                },
            };
            reset_snap(&mut ctx_x11, win);
        } else {
            let border_pixel = core
                .g
                .cfg
                .borderscheme
                .as_ref()
                .map(|s| s.normal.bg.pixel());
            if let Some(pixel) = border_pixel {
                let x11_win: Window = win.into();
                let _ = change_window_attributes(
                    x11.conn,
                    x11_win,
                    &ChangeWindowAttributesAux::new().border_pixel(Some(pixel)),
                );
                let _ = x11.conn.flush();
            }

            with_wm_ctx_x11(core, x11, |ctx| save_floating_win(ctx, win));

            if let Some(client) = core.g.clients.get_mut(&win) {
                client.snap_status = SnapPosition::Maximized;
            }

            let selmon_id = core.g.selected_monitor_id();
            with_wm_ctx_x11(core, x11, |ctx| arrange(ctx, Some(selmon_id)));
        }
    } else {
        with_wm_ctx_x11(core, x11, |ctx| toggle_floating(ctx));
    }
}
