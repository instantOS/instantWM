use crate::floating::{
    change_snap, reset_snap, save_floating_win, toggle_floating, SNAP_MAXIMIZED,
};
use crate::focus::direction_focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::monitor::arrange;
use crate::overlay::set_overlay_mode;
use crate::scratchpad::{hide_window, unhide_one};
use crate::types::*;
use crate::util::spawn;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

fn keycode_to_keysym(keycode: u8, _index: i32) -> u32 {
    0
}

pub const OVERLAY_BOTTOM: i32 = 2;
pub const OVERLAY_RIGHT: i32 = 1;

fn clean_mask(mask: u16) -> u16 {
    let globals = get_globals();
    let numlockmask = globals.numlockmask as u16;
    let lock_mask = ModMask::LOCK.bits();
    mask & !(numlockmask | lock_mask)
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
        let keysym = keycode_to_keysym(keycode, 0);

        let globals = get_globals();

        for key in &globals.keys {
            if keysym == key.keysym as u32
                && clean_mask(key.mod_mask as u16) == clean_mask(state.bits() as u16)
            {
                if let Some(func) = key.func {
                    drop(globals);
                    func(&key.arg);
                    return;
                }
            }
        }

        let has_sel = globals
            .selmon
            .and_then(|id| globals.monitors.get(id))
            .and_then(|m| m.sel)
            .is_some();
        drop(globals);

        if !has_sel {
            let globals = get_globals();
            for key in &globals.dkeys {
                if keysym == key.keysym as u32
                    && clean_mask(key.mod_mask as u16) == clean_mask(state.bits() as u16)
                {
                    if let Some(func) = key.func {
                        drop(globals);
                        func(&key.arg);
                        return;
                    }
                }
            }
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
        let keys = globals.keys.clone();
        let dkeys = globals.dkeys.clone();
        let free_alt_tab = true;

        let _ = ungrab_key(conn, 0, root, ModMask::ANY.into());

        let (keycode_min, keycode_max): (u8, u8) = (
            conn.setup().min_keycode as u8,
            conn.setup().max_keycode as u8,
        );

        let modifiers: [u16; 4] = [
            0,
            ModMask::LOCK.bits(),
            numlockmask as u16,
            (numlockmask as u16) | ModMask::LOCK.bits(),
        ];

        for keycode in keycode_min..=keycode_max {
            if keycode > 255 {
                continue;
            }

            for key in &keys {
                let keysym = keycode_to_keysym(keycode, 0);
                if keysym == key.keysym as u32 {
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

            let has_sel = globals
                .selmon
                .and_then(|id| globals.monitors.get(id))
                .and_then(|m| m.sel)
                .is_some();
            if !has_sel {
                for key in &dkeys {
                    let keysym = keycode_to_keysym(keycode, 0);
                    if keysym == key.keysym as u32 {
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

                for (i, keycode) in reply.keycodes.iter().enumerate() {
                    if *keycode != 0 {
                        let keysym = keycode_to_keysym(*keycode, 0);
                        if keysym == 0xff7f {
                            let mod_index = i / reply.keycodes_per_modifier() as usize;
                            if mod_index < 8 {
                                new_numlockmask = 1 << mod_index;
                            }
                        }
                    }
                }

                let mut globals = get_globals_mut();
                globals.numlockmask = new_numlockmask;
            }
        }
    }
}

pub fn up_press(_arg: &Arg) {
    let (sel_win, overlay_win, is_floating) = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                let sel = mon.sel;
                let overlay = mon.overlay;
                let is_floating = sel
                    .and_then(|w| globals.clients.get(&w).map(|c| c.isfloating))
                    .unwrap_or(false);
                (sel, overlay, is_floating)
            } else {
                return;
            }
        } else {
            return;
        }
    };

    if sel_win.is_none() {
        return;
    }

    if sel_win == overlay_win {
        set_overlay_mode(0);
        return;
    }

    if is_floating {
        toggle_floating(&Arg::default());
        return;
    }

    if let Some(win) = sel_win {
        hide_window(win);
    }
}

pub fn down_press(_arg: &Arg) {
    if unhide_one() {
        return;
    }

    let (sel_win, overlay_win, snapstatus) = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                let sel = mon.sel;
                let overlay = mon.overlay;
                let snapstatus = sel
                    .and_then(|w| globals.clients.get(&w).map(|c| c.snapstatus))
                    .unwrap_or(SnapPosition::None);
                (sel, overlay, snapstatus)
            } else {
                return;
            }
        } else {
            return;
        }
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
        set_overlay_mode(OVERLAY_BOTTOM);
        return;
    }

    let is_floating = {
        let globals = get_globals();
        sel_win
            .and_then(|w| globals.clients.get(&w).map(|c| c.isfloating))
            .unwrap_or(false)
    };

    if !is_floating {
        toggle_floating(&Arg::default());
    }
}

pub fn up_key(arg: &Arg) {
    let is_overview = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.sellt == 0
            } else {
                false
            }
        } else {
            false
        }
    };

    if is_overview {
        direction_focus(&Arg {
            ui: 0,
            ..Default::default()
        });
        return;
    }

    let has_tiling = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.sellt == 0
            } else {
                true
            }
        } else {
            true
        }
    };

    if !has_tiling {
        let sel_win = {
            let globals = get_globals();
            globals
                .selmon
                .and_then(|id| globals.monitors.get(id).and_then(|m| m.sel))
        };

        if let Some(win) = sel_win {
            let x11 = get_x11();
            if let Some(ref conn) = x11.conn {
                let globals = get_globals();
                if let Some(ref scheme) = globals.borderscheme {
                    let _ = change_window_attributes(
                        conn,
                        win,
                        &ChangeWindowAttributesAux::new()
                            .border_pixel(Some(scheme[0].pixel() as u32)),
                    );
                    let _ = conn.flush();
                }
            }
            change_snap(win, 0);
        }
        return;
    }

    focus_stack(arg);
}

pub fn down_key(arg: &Arg) {
    let is_overview = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.sellt == 0
            } else {
                false
            }
        } else {
            false
        }
    };

    if is_overview {
        direction_focus(&Arg {
            ui: 2,
            ..Default::default()
        });
        return;
    }

    let has_tiling = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.sellt == 0
            } else {
                true
            }
        } else {
            true
        }
    };

    if !has_tiling {
        let sel_win = {
            let globals = get_globals();
            globals
                .selmon
                .and_then(|id| globals.monitors.get(id).and_then(|m| m.sel))
        };

        if let Some(win) = sel_win {
            change_snap(win, 2);
        }
        return;
    }

    focus_stack(arg);
}

pub fn space_toggle(_arg: &Arg) {
    let has_tiling = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.sellt == 0
            } else {
                true
            }
        } else {
            true
        }
    };

    if !has_tiling {
        let sel_win = {
            let globals = get_globals();
            globals
                .selmon
                .and_then(|id| globals.monitors.get(id).and_then(|m| m.sel))
        };

        let Some(win) = sel_win else { return };

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
                            .border_pixel(Some(scheme[0].pixel() as u32)),
                    );
                    let _ = conn.flush();
                }
            }

            save_floating_win(win);

            let mut globals = get_globals_mut();
            if let Some(client) = globals.clients.get_mut(&win) {
                client.snapstatus = SnapPosition::Maximized;
            }
            drop(globals);

            if let Some(sel_mon_id) = get_globals().selmon {
                arrange(Some(sel_mon_id));
            }
        }
    } else {
        toggle_floating(&Arg::default());
    }
}

pub fn key_resize(arg: &Arg) {
    crate::floating::key_resize(arg);
}

pub fn center_window(arg: &Arg) {
    crate::floating::center_window(arg);
}

pub fn focus_stack(arg: &Arg) {
    let direction = arg.i;

    let (sel_win, clients_head) = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                (mon.sel, mon.clients)
            } else {
                return;
            }
        } else {
            return;
        }
    };

    let mut next: Option<Window> = None;
    let mut current = clients_head;
    let mut found_current = false;

    while let Some(c_win) = current {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&c_win) {
            let visible = crate::client::is_visible(c) && !crate::client::is_hidden(c_win);

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
                let visible = crate::client::is_visible(c) && !crate::client::is_hidden(c_win);
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

pub fn focus_mon(arg: &Arg) {
    crate::monitor::focus_mon(arg);
}

pub fn focus_nmon(arg: &Arg) {
    crate::monitor::focus_n_mon(arg);
}

pub fn follow_mon(arg: &Arg) {
    crate::monitor::follow_mon(arg);
}
