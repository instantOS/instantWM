use crate::client::{apply_size_hints, set_client_state, win_to_client};
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;
use x11rb::CURRENT_TIME;

const XEMBED_MAPPED: u32 = 1 << 0;
const XEMBED_WINDOW_ACTIVATE: u32 = 1;
const XEMBED_WINDOW_DEACTIVATE: u32 = 2;
const XEMBED_EMBEDDED_VERSION: u32 = 0;

pub fn get_systray_width() -> u32 {
    let globals = get_globals();
    if !globals.showsystray {
        return 1;
    }

    let mut w: u32 = 0;
    if let Some(ref systray) = globals.systray {
        for &icon_win in &systray.icons {
            if let Some(c) = globals.clients.get(&icon_win) {
                w += c.w as u32 + globals.systrayspacing;
            }
        }
    }

    if w > 0 {
        w + globals.systrayspacing
    } else {
        1
    }
}

pub fn remove_systray_icon(icon_win: Window) {
    let globals = get_globals();
    if !globals.showsystray {
        return;
    }

    drop(globals);

    let mut globals = get_globals_mut();
    if let Some(ref mut systray) = globals.systray {
        systray.icons.retain(|&w| w != icon_win);
    }

    globals.clients.remove(&icon_win);
}

pub fn update_systray_icon_geom(icon_win: Window, w: i32, h: i32) {
    let mut globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&icon_win) {
        client.h = globals.bh;
        if w == h {
            client.w = globals.bh;
        } else if h == globals.bh {
            client.w = w;
        } else {
            client.w = (globals.bh as f32 * (w as f32 / h as f32)) as i32;
        }

        let mut x = client.x;
        let mut y = client.y;
        let mut cw = client.w;
        let mut ch = client.h;
        let _ = apply_size_hints(client, &mut x, &mut y, &mut cw, &mut ch, false);

        if client.h > globals.bh {
            if client.w == client.h {
                client.w = globals.bh;
            } else {
                client.w = (globals.bh as f32 * (client.w as f32 / client.h as f32)) as i32;
            }
            client.h = globals.bh;
        }
    }
}

pub fn update_systray_icon_state(icon_win: Window, ev: &PropertyNotifyEvent) {
    let globals = get_globals();

    if !globals.showsystray {
        return;
    }

    let xembed_info_atom = globals.xatom[2];
    if ev.atom != xembed_info_atom {
        return;
    }

    drop(globals);

    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    let globals = get_globals();
    let flags = get_atom_prop(icon_win, xembed_info_atom);

    if flags == 0 {
        return;
    }

    let (current_tags, has_systray) = {
        if let Some(client) = globals.clients.get(&icon_win) {
            (client.tags, globals.systray.is_some())
        } else {
            return;
        }
    };

    drop(globals);

    if (flags & XEMBED_MAPPED) != 0 && current_tags == 0 {
        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&icon_win) {
            client.tags = 1;
        }

        let _ = conn.map_window(icon_win);
        let _ = conn.configure_window(
            icon_win,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
        set_client_state(icon_win, 1);

        let systray_win = globals.systray.as_ref().map(|s| s.win).unwrap_or(0);
        send_event(
            icon_win,
            xembed_info_atom,
            xembed_info_atom,
            CURRENT_TIME as i64,
            XEMBED_WINDOW_ACTIVATE as i64,
            0,
            systray_win as i64,
            XEMBED_EMBEDDED_VERSION as i64,
        );
    } else if (flags & XEMBED_MAPPED) == 0 && current_tags != 0 {
        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&icon_win) {
            client.tags = 0;
        }

        let _ = conn.unmap_window(icon_win);
        set_client_state(icon_win, 0);

        let systray_win = globals.systray.as_ref().map(|s| s.win).unwrap_or(0);
        send_event(
            icon_win,
            xembed_info_atom,
            xembed_info_atom,
            CURRENT_TIME as i64,
            XEMBED_WINDOW_DEACTIVATE as i64,
            0,
            systray_win as i64,
            XEMBED_EMBEDDED_VERSION as i64,
        );
    }
}

pub fn update_systray() {
    let globals = get_globals();

    if !globals.showsystray {
        return;
    }

    let m = systray_to_mon(None);
    let mon = match globals.monitors.get(m) {
        Some(mon) => mon,
        None => return,
    };

    let x = mon.mx + mon.mw;
    let mut w: u32 = 1;

    drop(globals);

    let systray_exists = {
        let globals = get_globals();
        globals.systray.is_some()
    };

    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    if !systray_exists {
        let globals = get_globals();
        let root = globals.root;
        let bh = globals.bh;
        let by = mon.by;

        let net_system_tray = globals.netatom[NetAtom::SystemTray as usize];
        let net_system_tray_horz = globals.netatom[NetAtom::SystemTrayOrientationHorz as usize];

        let bg_pixel = if let Some(ref scheme) = globals.statusscheme {
            scheme[1].pixel
        } else {
            0
        };

        drop(globals);

        let systray_win = conn.generate_id().ok();
        let Ok(systray_win) = systray_win else { return };

        let result = conn.create_window(
            x11rb::COPY_FROM_PARENT.into(),
            systray_win,
            root,
            x,
            by,
            w as u16,
            bh as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT.into(),
            &CreateWindowAux::new()
                .event_mask(EventMask::BUTTON_PRESS | EventMask::EXPOSURE)
                .override_redirect(1)
                .background_pixel(bg_pixel),
        );

        if result.is_err() {
            return;
        }

        let _ = conn.change_property32(
            PropMode::REPLACE,
            systray_win,
            net_system_tray,
            AtomEnum::CARDINAL.into(),
            &[net_system_tray_horz],
        );

        conn.select_input(systray_win, EventMask::SUBSTRUCTURE_NOTIFY)
            .ok();

        let _ = conn.map_window(systray_win);
        let _ = conn.change_window_attributes(
            systray_win,
            &ChangeWindowAttributesAux::new().background_pixel(bg_pixel),
        );

        let _ = conn.set_selection_owner(systray_win, net_system_tray, CURRENT_TIME);

        {
            let mut globals = get_globals_mut();
            globals.systray = Some(Systray {
                win: systray_win,
                icons: Vec::new(),
            });
        }

        let globals = get_globals();
        let manager_atom = globals.xatom[0];
        send_event(
            root,
            manager_atom,
            manager_atom,
            CURRENT_TIME as i64,
            net_system_tray as i64,
            systray_win as i64,
            0,
            0,
        );

        let _ = conn.flush();
    }

    let globals = get_globals();
    let systray = match &globals.systray {
        Some(s) => s,
        None => return,
    };

    let bh = globals.bh;
    let systrayspacing = globals.systrayspacing;
    let bg_pixel = globals
        .statusscheme
        .as_ref()
        .map(|s| s[1].pixel)
        .unwrap_or(0);

    w = 0;
    for &icon_win in &systray.icons {
        let _ = conn.change_window_attributes(
            icon_win,
            &ChangeWindowAttributesAux::new().background_pixel(bg_pixel),
        );
        let _ = conn.map_window(icon_win);

        w += systrayspacing;

        if let Some(client) = globals.clients.get(&icon_win) {
            let icon_w = client.w;
            let icon_h = client.h;

            let _ = conn.configure_window(
                icon_win,
                &ConfigureWindowAux::new()
                    .x(w as i32)
                    .y(0)
                    .width(icon_w as u32)
                    .height(icon_h as u32),
            );

            w += icon_w as u32;
        }
    }

    let systray_win = systray.win;
    let barwin = mon.barwin;
    let by = mon.by;

    drop(globals);

    w = if w > 0 { w + systrayspacing } else { 1 };
    let x = x - w as i32;

    let _ = conn.configure_window(
        systray_win,
        &ConfigureWindowAux::new()
            .x(x)
            .y(by)
            .width(w)
            .height(bh as u32),
    );

    let _ = conn.configure_window(
        systray_win,
        &ConfigureWindowAux::new()
            .stack_mode(StackMode::ABOVE)
            .sibling(barwin),
    );

    let _ = conn.map_window(systray_win);

    let _ = conn.flush();
}

pub fn win_to_systray_icon(win: Window) -> Option<Window> {
    let globals = get_globals();
    if !globals.showsystray {
        return None;
    }

    if let Some(ref systray) = globals.systray {
        for &icon_win in &systray.icons {
            if icon_win == win {
                return Some(win);
            }
        }
    }
    None
}

pub fn systray_to_mon(m: Option<MonitorId>) -> MonitorId {
    let globals = get_globals();

    if globals.systraypinning == 0 {
        return match m {
            Some(id) => {
                if id == globals.selmon.unwrap_or(0) {
                    id
                } else {
                    globals.selmon.unwrap_or(0)
                }
            }
            None => globals.selmon.unwrap_or(0),
        };
    }

    let n = globals.monitors.len();
    let target = (globals.systraypinning as usize).min(n);

    if globals.systraypinning as usize > n {
        0
    } else {
        target.saturating_sub(1)
    }
}

fn get_atom_prop(win: Window, atom: u32) -> u32 {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        if let Ok(cookie) = conn.get_property(false, win, atom, AtomEnum::CARDINAL, 0, 2) {
            if let Ok(reply) = cookie.reply() {
                if let Some(val) = reply.value32().and_then(|mut v| v.next()) {
                    return val;
                }
            }
        }
    }
    0
}

fn send_event(win: Window, proto: u32, mask: u32, d0: i64, d1: i64, d2: i64, d3: i64, d4: i64) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let event = ClientMessageEvent {
            response_type: CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window: win,
            type_: proto,
            data: ClientMessageData::from([d0 as u32, d1 as u32, d2 as u32, d3 as u32, d4 as u32]),
        };
        let _ = conn.send_event(false, win, EventMask::from(mask), &event);
        let _ = conn.flush();
    }
}
