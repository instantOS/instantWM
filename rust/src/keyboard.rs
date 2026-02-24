use crate::floating::{change_snap, reset_snap, save_floating_win, toggle_floating, SnapDir};
use crate::focus::direction_focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::layouts::arrange;
use crate::overlay::set_overlay_mode;
use crate::scratchpad::unhide_one;
use crate::types::Direction;
use crate::types::*;
use crate::util::get_sel_win;
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

pub fn key_press(e: &KeyPressEvent) {
    let keycode = e.detail;
    let state = e.state;

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let keysym = keycode_to_keysym(conn, keycode, 0);

        let matching_key = {
            let globals = get_globals();
            let numlockmask = globals.numlockmask;
            let mut result = None;
            for key in &globals.keys {
                if keysym == key.keysym
                    && clean_mask(key.mod_mask as u16, numlockmask)
                        == clean_mask(state.bits(), numlockmask)
                {
                    result = Some(key);
                    break;
                }
            }
            if result.is_none() && get_sel_win().is_none() {
                for key in &globals.dkeys {
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
            (key.action)();
        }
    }
}

pub fn key_release(_e: &KeyReleaseEvent) {}

pub fn grab_keys() {
    update_num_lock_mask();

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let root = globals.root;
        let numlockmask = globals.numlockmask;
        let keys = globals.keys.as_slice();
        let dkeys = globals.dkeys.as_slice();
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

            if get_sel_win().is_none() {
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

pub fn update_num_lock_mask() {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
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

                let globals = get_globals_mut();
                globals.numlockmask = new_numlockmask;
            }
        }
    }
}

pub fn up_press() {
    let (sel_win, overlay_win, is_floating) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        let sel = mon.sel;
        let overlay = mon.overlay;
        let is_floating = sel
            .and_then(|w| globals.clients.get(&w).map(|c| c.isfloating))
            .unwrap_or(false);
        (sel, overlay, is_floating)
    };

    if sel_win.is_none() {
        return;
    }

    if sel_win == overlay_win {
        set_overlay_mode(OverlayMode::Top);
        return;
    }

    if is_floating {
        toggle_floating();
        return;
    }

    if let Some(win) = sel_win {
        crate::client::hide(win);
    }
}

pub fn down_press() {
    if unhide_one() {
        return;
    }

    let (sel_win, overlay_win, snapstatus) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        let sel = mon.sel;
        let overlay = mon.overlay;
        let snapstatus = sel
            .and_then(|w| globals.clients.get(&w).map(|c| c.snapstatus))
            .unwrap_or(SnapPosition::None);
        (sel, overlay, snapstatus)
    };

    if sel_win.is_none() {
        return;
    }

    if snapstatus != SnapPosition::None {
        if let Some(win) = sel_win {
            reset_snap(win);
        }
        return;
    }

    if sel_win == overlay_win {
        set_overlay_mode(OverlayMode::Bottom);
        return;
    }

    let is_floating = {
        let globals = get_globals();
        sel_win
            .and_then(|w| globals.clients.get(&w).map(|c| c.isfloating))
            .unwrap_or(false)
    };

    if !is_floating {
        toggle_floating();
    }
}

pub fn up_key(direction: i32) {
    let is_overview = {
        let globals = get_globals();
        globals
            .monitors
            .get(globals.selmon)
            .map(|mon| crate::monitor::is_current_layout_tiling(mon, &globals.tags))
            .unwrap_or(false)
    };

    if is_overview {
        direction_focus(Direction::Up);
        return;
    }

    let has_tiling = {
        let globals = get_globals();
        globals
            .monitors
            .get(globals.selmon)
            .map(|mon| crate::monitor::is_current_layout_tiling(mon, &globals.tags))
            .unwrap_or(true)
    };

    if !has_tiling {
        if let Some(win) = get_sel_win() {
            let x11 = get_x11();
            if let Some(ref conn) = x11.conn {
                let globals = get_globals();
                if let Some(ref scheme) = globals.borderscheme {
                    let _ = change_window_attributes(
                        conn,
                        win,
                        &ChangeWindowAttributesAux::new()
                            .border_pixel(Some(scheme.normal.bg.pixel())),
                    );
                    let _ = conn.flush();
                }
            }
            change_snap(win, SnapDir::Up);
        }
        return;
    }

    focus_stack(direction);
}

pub fn down_key(direction: i32) {
    let is_overview = {
        let globals = get_globals();
        globals
            .monitors
            .get(globals.selmon)
            .map(|mon| crate::monitor::is_current_layout_tiling(mon, &globals.tags))
            .unwrap_or(false)
    };

    if is_overview {
        direction_focus(Direction::Down);
        return;
    }

    let has_tiling = {
        let globals = get_globals();
        globals
            .monitors
            .get(globals.selmon)
            .map(|mon| crate::monitor::is_current_layout_tiling(mon, &globals.tags))
            .unwrap_or(true)
    };

    if !has_tiling {
        if let Some(win) = get_sel_win() {
            change_snap(win, SnapDir::Down);
        }
        return;
    }

    focus_stack(direction);
}

pub fn space_toggle() {
    let has_tiling = {
        let globals = get_globals();
        globals
            .monitors
            .get(globals.selmon)
            .map(|mon| crate::monitor::is_current_layout_tiling(mon, &globals.tags))
            .unwrap_or(true)
    };

    if !has_tiling {
        let Some(win) = get_sel_win() else { return };

        let snapstatus = {
            let globals = get_globals();
            globals
                .clients
                .get(&win)
                .map(|c| c.snapstatus)
                .unwrap_or(SnapPosition::None)
        };

        if snapstatus != SnapPosition::None {
            reset_snap(win);
        } else {
            let x11 = get_x11();
            if let Some(ref conn) = x11.conn {
                let globals = get_globals();
                if let Some(ref scheme) = globals.borderscheme {
                    let _ = change_window_attributes(
                        conn,
                        win,
                        &ChangeWindowAttributesAux::new()
                            .border_pixel(Some(scheme.normal.bg.pixel())),
                    );
                    let _ = conn.flush();
                }
            }

            save_floating_win(win);

            let globals = get_globals_mut();
            if let Some(client) = globals.clients.get_mut(&win) {
                client.snapstatus = SnapPosition::Maximized;
            }

            arrange(Some(get_globals().selmon));
        }
    } else {
        toggle_floating();
    }
}

pub fn key_resize(dir: CardinalDirection) {
    crate::floating::key_resize(dir);
}

pub fn center_window() {
    crate::floating::center_window();
}

pub fn focus_stack(direction: i32) {
    let (sel_win, clients_head) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        (mon.sel, mon.clients)
    };

    let mut next: Option<Window> = None;
    let mut current = clients_head;
    let mut found_current = false;

    while let Some(c_win) = current {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&c_win) {
            let visible = c.is_visible() && !c.is_hidden;

            if found_current && visible && !c.isfloating {
                next = Some(c_win);
                break;
            }

            if Some(c_win) == sel_win {
                found_current = true;
            }

            current = c.next;
        } else {
            break;
        }
    }

    if next.is_none() && direction > 0 {
        current = clients_head;
        while let Some(c_win) = current {
            let globals = get_globals();
            if let Some(c) = globals.clients.get(&c_win) {
                let visible = c.is_visible() && !c.is_hidden;
                if visible && !c.isfloating {
                    next = Some(c_win);
                    break;
                }
                current = c.next;
            } else {
                break;
            }
        }
    }

    if let Some(win) = next {
        crate::focus::focus(Some(win));
    }
}

pub fn focus_mon(direction: i32) {
    crate::monitor::focus_mon(direction);
}

pub fn focus_nmon(index: i32) {
    crate::monitor::focus_n_mon(index);
}

pub fn follow_mon(direction: i32) {
    crate::monitor::follow_mon(direction);
}
