use crate::contexts::WmCtx;
use crate::floating::{change_snap, reset_snap, save_floating_win, toggle_floating, SnapDir};
use crate::focus::direction_focus;
use crate::layouts::arrange;
use crate::overlay::set_overlay_mode;
use crate::scratchpad::unhide_one;
use crate::types::Direction;
use crate::types::*;
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

pub fn key_press(ctx: &mut WmCtx, e: &KeyPressEvent) {
    let keycode = e.detail;
    let state = e.state;

    if let Some(ref conn) = ctx.x11.conn {
        let keysym = keycode_to_keysym(conn, keycode, 0);

        let matching_key = {
            let numlockmask = ctx.g.cfg.numlockmask;
            let mut result = None;
            for key in &ctx.g.cfg.keys {
                if keysym == key.keysym
                    && clean_mask(key.mod_mask as u16, numlockmask)
                        == clean_mask(state.bits(), numlockmask)
                {
                    result = Some(key);
                    break;
                }
            }
            let sel_win = ctx.g.monitors.get(ctx.g.selmon).and_then(|mon| mon.sel);
            if result.is_none() && sel_win.is_none() {
                for key in &ctx.g.cfg.dkeys {
                    if keysym == key.keysym
                        && clean_mask(key.mod_mask as u16, numlockmask)
                            == clean_mask(state.bits(), numlockmask)
                    {
                        result = Some(key);
                        break;
                    }
                }
            }
            result
        };

        if let Some(key) = matching_key {
            (key.action)(ctx);
        }
    }
}

pub fn key_release(_ctx: &mut WmCtx, _e: &KeyReleaseEvent) {}

pub fn grab_keys(ctx: &WmCtx) {
    if let Some(ref conn) = ctx.x11.conn {
        let root = ctx.g.cfg.root;
        let numlockmask = ctx.g.cfg.numlockmask;
        let keys = ctx.g.cfg.keys.as_slice();
        let dkeys = ctx.g.cfg.dkeys.as_slice();
        let free_alt_tab = true;

        let _ = ungrab_key(conn, 0, root, ModMask::ANY);

        let (keycode_min, keycode_max): (u8, u8) =
            (conn.setup().min_keycode, conn.setup().max_keycode);

        let modifiers: [u16; 4] = [
            0,
            ModMask::LOCK.bits(),
            numlockmask as u16,
            (numlockmask as u16) | ModMask::LOCK.bits(),
        ];

        let mapping = conn
            .get_keyboard_mapping(keycode_min, keycode_max - keycode_min + 1)
            .unwrap()
            .reply()
            .unwrap();

        let get_keysym = |keycode: u8| -> u32 {
            let index = (keycode - keycode_min) as usize * mapping.keysyms_per_keycode as usize;
            if index < mapping.keysyms.len() {
                mapping.keysyms[index]
            } else {
                0
            }
        };

        for keycode in keycode_min..=keycode_max {
            if keycode > 255 {
                continue;
            }

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

            let sel_win = ctx.g.monitors.get(ctx.g.selmon).and_then(|mon| mon.sel);
            if sel_win.is_none() {
                for key in dkeys {
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
}

pub fn update_num_lock_mask(ctx: &mut WmCtx) {
    if let Some(ref conn) = ctx.x11.conn {
        let modmap = conn.get_modifier_mapping();
        if let Ok(cookie) = modmap {
            if let Ok(reply) = cookie.reply() {
                let mut new_numlockmask: u32 = 0;

                let (keycode_min, keycode_max) =
                    (conn.setup().min_keycode, conn.setup().max_keycode);
                let mapping = conn
                    .get_keyboard_mapping(keycode_min, keycode_max - keycode_min + 1)
                    .unwrap()
                    .reply()
                    .unwrap();

                for (i, keycode) in reply.keycodes.iter().enumerate() {
                    if *keycode >= keycode_min && *keycode <= keycode_max {
                        let idx = (*keycode - keycode_min) as usize
                            * mapping.keysyms_per_keycode as usize;
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

                ctx.g.cfg.numlockmask = new_numlockmask;
            }
        }
    }
}

pub fn up_press(ctx: &mut WmCtx) {
    let (sel_win, overlay_win, is_floating) = {
        let Some(mon) = ctx.g.monitors.get(ctx.g.selmon) else {
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
        crate::client::hide(win);
    }
}

pub fn down_press(ctx: &mut WmCtx) {
    if unhide_one() {
        return;
    }

    let (sel_win, overlay_win, snapstatus, is_floating) = {
        let Some(mon) = ctx.g.monitors.get(ctx.g.selmon) else {
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

pub fn up_key(ctx: &mut WmCtx, direction: i32) {
    let is_overview = {
        ctx.g
            .monitors
            .get(ctx.g.selmon)
            .map(|mon| crate::monitor::is_current_layout_tiling(mon, &ctx.g.tags))
            .unwrap_or(false)
    };

    if is_overview {
        direction_focus(Direction::Up);
        return;
    }

    let has_tiling = {
        ctx.g
            .monitors
            .get(ctx.g.selmon)
            .map(|mon| crate::monitor::is_current_layout_tiling(mon, &ctx.g.tags))
            .unwrap_or(true)
    };

    if !has_tiling {
        if let Some(win) = ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.sel) {
            if let Some(ref conn) = ctx.x11.conn {
                if let Some(ref scheme) = ctx.g.cfg.borderscheme {
                    let _ = change_window_attributes(
                        conn,
                        win,
                        &ChangeWindowAttributesAux::new()
                            .border_pixel(Some(scheme.normal.bg.pixel())),
                    );
                    let _ = conn.flush();
                }
            }
            change_snap(ctx, win, SnapDir::Up);
        }
        return;
    }

    focus_stack(ctx, direction);
}

//TODO: this should use the direction enum
pub fn down_key(ctx: &mut WmCtx, direction: i32) {
    let is_overview = {
        ctx.g
            .monitors
            .get(ctx.g.selmon)
            .map(|mon| crate::monitor::is_current_layout_tiling(mon, &ctx.g.tags))
            .unwrap_or(false)
    };

    if is_overview {
        direction_focus(Direction::Down);
        return;
    }

    let has_tiling = {
        ctx.g
            .monitors
            .get(ctx.g.selmon)
            .map(|mon| crate::monitor::is_current_layout_tiling(mon, &ctx.g.tags))
            .unwrap_or(true)
    };

    if !has_tiling {
        if let Some(win) = ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.sel) {
            change_snap(ctx, win, SnapDir::Down);
        }
        return;
    }

    focus_stack(ctx, direction);
}

pub fn space_toggle(ctx: &mut WmCtx) {
    let has_tiling = {
        ctx.g
            .monitors
            .get(ctx.g.selmon)
            .map(|mon| crate::monitor::is_current_layout_tiling(mon, &ctx.g.tags))
            .unwrap_or(true)
    };

    if !has_tiling {
        let Some(win) = ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.sel) else {
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
            if let Some(ref conn) = ctx.x11.conn {
                if let Some(ref scheme) = ctx.g.cfg.borderscheme {
                    let _ = change_window_attributes(
                        conn,
                        win,
                        &ChangeWindowAttributesAux::new()
                            .border_pixel(Some(scheme.normal.bg.pixel())),
                    );
                    let _ = conn.flush();
                }
            }

            save_floating_win(ctx, win);

            if let Some(client) = ctx.g.clients.get_mut(&win) {
                client.snapstatus = SnapPosition::Maximized;
            }

            arrange(ctx, Some(ctx.g.selmon));
        }
    } else {
        toggle_floating(ctx);
    }
}

pub fn key_resize(ctx: &mut WmCtx, win: Window, dir: CardinalDirection) {
    crate::floating::key_resize(ctx, win, dir);
}

pub fn center_window(ctx: &mut WmCtx, win: Window) {
    crate::floating::center_window(ctx, win);
}

pub fn focus_stack(ctx: &mut WmCtx, direction: i32) {
    let (sel_win, clients_head) = {
        let Some(mon) = ctx.g.monitors.get(ctx.g.selmon) else {
            return;
        };
        (mon.sel, mon.clients)
    };

    let mut next: Option<Window> = None;
    let mut current = clients_head;
    let mut found_current = false;

    while let Some(c_win) = current {
        let (next_client, visible, is_floating) = match ctx.g.clients.get(&c_win) {
            Some(c) => (c.next, c.is_visible() && !c.is_hidden, c.isfloating),
            None => break,
        };

        if found_current && visible && !is_floating {
            next = Some(c_win);
            break;
        }

        if Some(c_win) == sel_win {
            found_current = true;
        }

        current = next_client;
    }

    if next.is_none() && direction > 0 {
        current = clients_head;
        while let Some(c_win) = current {
            let (next_client, visible, is_floating) = match ctx.g.clients.get(&c_win) {
                Some(c) => (c.next, c.is_visible() && !c.is_hidden, c.isfloating),
                None => break,
            };
            if visible && !is_floating {
                next = Some(c_win);
                break;
            }
            current = next_client;
        }
    }

    if let Some(win) = next {
        crate::focus::focus(ctx, Some(win));
    }
}

pub fn focus_mon(ctx: &mut WmCtx, direction: i32) {
    crate::monitor::focus_mon(ctx, direction);
}

pub fn focus_nmon(ctx: &mut WmCtx, index: i32) {
    crate::monitor::focus_n_mon(ctx, index);
}

pub fn follow_mon(ctx: &mut WmCtx, direction: i32) {
    crate::monitor::follow_mon(ctx, direction);
}
