use crate::animation::animate_client;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::monitor::arrange;
use crate::types::*;
use crate::util::{max, min};
use std::sync::atomic::{AtomicU32, Ordering};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;
use x11rb::CURRENT_TIME;

/// The window currently being animated (0 = none).
pub static ANIM_CLIENT: AtomicU32 = AtomicU32::new(0);
/// The previously focused window (0 = none), used by focus-last-client.
pub static LAST_CLIENT: AtomicU32 = AtomicU32::new(0);
pub const BROKEN: &str = "broken";

pub const WM_STATE_NORMAL: i32 = 1;
pub const WM_STATE_ICONIC: i32 = 3;
pub const WM_STATE_WITHDRAWN: i32 = 0;

pub const MWM_HINTS_FLAGS_FIELD: usize = 0;
pub const MWM_HINTS_DECORATIONS_FIELD: usize = 2;
pub const MWM_HINTS_DECORATIONS: u32 = 1 << 1;
pub const MWM_DECOR_ALL: u32 = 1 << 0;
pub const MWM_DECOR_BORDER: u32 = 1 << 1;
pub const MWM_DECOR_TITLE: u32 = 1 << 3;

pub fn attach(win: Window) {
    let mut globals = get_globals_mut();
    let mon_id = globals.clients.get(&win).and_then(|c| c.mon_id);
    if let Some(mon_id) = mon_id {
        let old_head = globals.monitors.get(mon_id).and_then(|m| m.clients);
        if let Some(client) = globals.clients.get_mut(&win) {
            client.next = old_head;
        }
        if let Some(mon) = globals.monitors.get_mut(mon_id) {
            mon.clients = Some(win);
        }
    }
}

pub fn attach_stack(win: Window) {
    let mut globals = get_globals_mut();
    let mon_id = globals.clients.get(&win).and_then(|c| c.mon_id);
    if let Some(mon_id) = mon_id {
        let old_stack = globals.monitors.get(mon_id).and_then(|m| m.stack);
        if let Some(client) = globals.clients.get_mut(&win) {
            client.snext = old_stack;
        }
        if let Some(mon) = globals.monitors.get_mut(mon_id) {
            mon.stack = Some(win);
        }
    }
}

pub fn detach(win: Window) {
    let mut globals = get_globals_mut();
    let mon_id = {
        if let Some(client) = globals.clients.get(&win) {
            client.mon_id
        } else {
            return;
        }
    };

    if let Some(mid) = mon_id {
        let client_next = globals.clients.get(&win).and_then(|c| c.next);

        let mut traversal: Vec<(Window, Option<Window>, Option<Window>)> = Vec::new();
        let mut current = globals.monitors[mid].clients;
        let mut prev: Option<Window> = None;

        while let Some(cur_win) = current {
            let next = globals.clients.get(&cur_win).and_then(|c| c.next);
            traversal.push((cur_win, prev, next));
            prev = Some(cur_win);
            current = next;
        }

        for (cur_win, prev_win, _next) in traversal {
            if cur_win == win {
                if let Some(prev_win) = prev_win {
                    if let Some(prev_client) = globals.clients.get_mut(&prev_win) {
                        prev_client.next = client_next;
                    }
                } else {
                    globals.monitors[mid].clients = client_next;
                }
                return;
            }
        }
    }
}

pub fn detach_stack(win: Window) {
    let mut globals = get_globals_mut();
    let mon_id = {
        if let Some(client) = globals.clients.get(&win) {
            client.mon_id
        } else {
            return;
        }
    };

    if let Some(mid) = mon_id {
        let client_snext = globals.clients.get(&win).and_then(|c| c.snext);

        let mut traversal: Vec<(Window, Option<Window>, Option<Window>)> = Vec::new();
        let mut current = globals.monitors[mid].stack;
        let mut prev: Option<Window> = None;

        while let Some(cur_win) = current {
            let snext = globals.clients.get(&cur_win).and_then(|c| c.snext);
            traversal.push((cur_win, prev, snext));
            prev = Some(cur_win);
            current = snext;
        }

        for (cur_win, prev_win, _snext) in traversal {
            if cur_win == win {
                if let Some(prev_win) = prev_win {
                    if let Some(prev_client) = globals.clients.get_mut(&prev_win) {
                        prev_client.snext = client_snext;
                    }
                } else {
                    globals.monitors[mid].stack = client_snext;
                }

                if globals.monitors[mid].sel == Some(win) {
                    let mut t = globals.monitors[mid].stack;
                    while let Some(t_win) = t {
                        let t_snext = globals.clients.get(&t_win).and_then(|tc| tc.snext);
                        let is_vis = globals
                            .clients
                            .get(&t_win)
                            .map(|tc| is_visible(tc))
                            .unwrap_or(false);
                        let is_hid = is_hidden(t_win);
                        if is_vis && !is_hid {
                            globals.monitors[mid].sel = Some(t_win);
                            return;
                        }
                        t = t_snext;
                    }
                    globals.monitors[mid].sel = None;
                }
                return;
            }
        }
    }
}

pub fn is_visible(c: &Client) -> bool {
    if c.issticky {
        return true;
    }
    if let Some(mon_id) = c.mon_id {
        let globals = get_globals();
        if let Some(mon) = globals.monitors.get(mon_id) {
            let tags = mon.tagset[mon.seltags as usize];
            return (c.tags & tags) != 0;
        }
    }
    false
}

pub fn get_state(win: Window) -> i32 {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        if let Ok(cookie) =
            conn.get_property(false, win, globals.wmatom.state, globals.wmatom.state, 0, 2)
        {
            if let Ok(reply) = cookie.reply() {
                if let Some(mut data) = reply.value32() {
                    return data.next().unwrap_or(WM_STATE_NORMAL as u32) as i32;
                }
            }
        }
    }
    WM_STATE_NORMAL
}

pub fn client_width(c: &Client) -> i32 {
    c.geo.w + 2 * c.border_width
}

pub fn client_height(c: &Client) -> i32 {
    c.geo.h + 2 * c.border_width
}

pub fn next_tiled(start_win: Option<Window>) -> Option<Window> {
    let mut current = start_win;
    let globals = get_globals();

    while let Some(win) = current {
        if let Some(c) = globals.clients.get(&win) {
            if !c.isfloating && is_visible(c) && !is_hidden(win) {
                return Some(win);
            }
            current = c.next;
        } else {
            break;
        }
    }
    None
}

pub fn pop(win: Window) {
    detach(win);
    attach(win);
    crate::focus::focus(Some(win));
    let mon_id = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            c.mon_id
        } else {
            None
        }
    };
    if let Some(mid) = mon_id {
        arrange(Some(mid));
    }
}

pub fn win_to_client(w: Window) -> Option<Window> {
    let globals = get_globals();
    for (&win, _c) in globals.clients.iter() {
        if win == w {
            return Some(win);
        }
    }
    None
}

pub fn set_client_state(win: Window, state: i32) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let data: [u8; 8] = unsafe { std::mem::transmute([state as u32, 0u32]) };
        let _ = conn.change_property(
            PropMode::REPLACE,
            win,
            globals.wmatom.state,
            globals.wmatom.state,
            8u8,
            data.len() as u32,
            &data,
        );
        let _ = conn.flush();
    }
}

pub fn set_client_tag_prop(win: Window) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            let mon_num = if let Some(mon_id) = c.mon_id {
                globals.monitors[mon_id].num as u32
            } else {
                0
            };
            let data: [u8; 8] = unsafe { std::mem::transmute([c.tags, mon_num]) };
            let _ = conn.change_property(
                PropMode::REPLACE,
                win,
                globals.netatom.client_info,
                AtomEnum::CARDINAL,
                8u8,
                data.len() as u32,
                &data,
            );
            let _ = conn.flush();
        }
    }
}

pub fn send_event(
    win: Window,
    proto: u32,
    mask: u32,
    d0: i64,
    d1: i64,
    d2: i64,
    d3: i64,
    d4: i64,
) -> bool {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let wmatom_protocols = globals.wmatom.protocols;
        let wmatom_take_focus = globals.wmatom.take_focus;
        let wmatom_delete = globals.wmatom.delete;

        let (exists, message_type) = if proto == wmatom_take_focus || proto == wmatom_delete {
            let mut exists = false;
            if let Ok(cookie) = get_wm_protocols(conn, win) {
                if let Ok(reply) = cookie.reply() {
                    if let Some(atoms) = reply.value32() {
                        for p in atoms {
                            if p == proto {
                                exists = true;
                                break;
                            }
                        }
                    }
                }
            }
            (exists, wmatom_protocols)
        } else {
            (true, proto)
        };

        if exists {
            let event = ClientMessageEvent {
                response_type: CLIENT_MESSAGE_EVENT,
                format: 32,
                sequence: 0,
                window: win,
                type_: message_type,
                data: ClientMessageData::from([
                    d0 as u32, d1 as u32, d2 as u32, d3 as u32, d4 as u32,
                ]),
            };
            let _ = conn.send_event(false, win, EventMask::from(mask), &event);
            let _ = conn.flush();
        }
        exists
    } else {
        false
    }
}

fn get_wm_protocols(
    conn: &x11rb::rust_connection::RustConnection,
    win: Window,
) -> Result<
    x11rb::cookie::Cookie<'_, x11rb::rust_connection::RustConnection, GetPropertyReply>,
    x11rb::errors::ConnectionError,
> {
    let globals = get_globals();
    conn.get_property(
        false,
        win,
        globals.wmatom.protocols,
        AtomEnum::ATOM,
        0,
        1024,
    )
}

pub fn configure(win: Window) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            let event = ConfigureNotifyEvent {
                response_type: CONFIGURE_NOTIFY_EVENT,
                sequence: 0,
                event: win,
                window: win,
                above_sibling: 0,
                x: c.geo.x as i16,
                y: c.geo.y as i16,
                width: c.geo.w as u16,
                height: c.geo.h as u16,
                border_width: c.border_width as u16,
                override_redirect: false,
            };
            let _ = conn.send_event(false, win, EventMask::STRUCTURE_NOTIFY, &event);
            let _ = conn.flush();
        }
    }
}

pub fn set_focus(win: Window) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            if !c.neverfocus {
                let _ = conn.set_input_focus(InputFocus::POINTER_ROOT, win, CURRENT_TIME);
                let _ = conn.change_property32(
                    PropMode::REPLACE,
                    globals.root,
                    globals.netatom.active_window,
                    AtomEnum::WINDOW,
                    &[win],
                );
            }
            send_event(
                win,
                globals.wmatom.take_focus,
                0,
                globals.wmatom.take_focus as i64,
                CURRENT_TIME as i64,
                0,
                0,
                0,
            );
            let _ = conn.flush();
        }
    }
}

pub fn unfocus_win(win: Window, set_focus: bool) {
    if win == 0 {
        return;
    }
    LAST_CLIENT.store(win, Ordering::Relaxed);
    grab_buttons(win, false);

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        if let Some(ref scheme) = globals.borderscheme {
            let clr = &scheme.normal.bg;
            let _ = conn.change_window_attributes(
                win,
                &ChangeWindowAttributesAux::new().border_pixel(clr.pixel()),
            );
        }
        if set_focus {
            let _ = conn.set_input_focus(InputFocus::POINTER_ROOT, globals.root, CURRENT_TIME);
            let _ = conn.delete_property(globals.root, globals.netatom.active_window);
        }
        let _ = conn.flush();
    }
}

pub fn show_hide(win: Option<Window>) {
    let current = match win {
        Some(w) => w,
        None => return,
    };

    let globals = get_globals();
    if let Some(c) = globals.clients.get(&current) {
        let is_vis = is_visible(c);
        let snext = c.snext;

        drop(globals);

        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            if is_vis {
                let (x, y) = {
                    let globals = get_globals();
                    if let Some(client) = globals.clients.get(&current) {
                        (client.geo.x, client.geo.y)
                    } else {
                        return;
                    }
                };
                let _ = conn.configure_window(current, &ConfigureWindowAux::new().x(x).y(y));
                let _ = conn.flush();

                let (is_floating, is_fullscreen, is_fake_fullscreen, mon_id, x, y, w, h) = {
                    let globals = get_globals();
                    if let Some(client) = globals.clients.get(&current) {
                        (
                            client.isfloating,
                            client.is_fullscreen,
                            client.isfakefullscreen,
                            client.mon_id,
                            client.geo.x,
                            client.geo.y,
                            client.geo.w,
                            client.geo.h,
                        )
                    } else {
                        return;
                    }
                };

                let has_arrange = if let Some(mid) = mon_id {
                    let globals = get_globals();
                    if let Some(mon) = globals.monitors.get(mid) {
                        mon.sellt == 0
                    } else {
                        false
                    }
                } else {
                    false
                };

                if (!has_arrange || is_floating) && (!is_fullscreen || is_fake_fullscreen) {
                    resize(current, x, y, w, h, false);
                }
                show_hide(snext);
            } else {
                show_hide(snext);
                let w_val = {
                    let globals = get_globals();
                    if let Some(client) = globals.clients.get(&current) {
                        client_width(client)
                    } else {
                        0
                    }
                };
                let y = {
                    let globals = get_globals();
                    if let Some(client) = globals.clients.get(&current) {
                        client.geo.y
                    } else {
                        0
                    }
                };
                let _ =
                    conn.configure_window(current, &ConfigureWindowAux::new().x(-2 * w_val).y(y));
                let _ = conn.flush();
            }
        }
    }
}

pub fn show(win: Window) {
    let globals = get_globals();
    let client = match globals.clients.get(&win) {
        Some(c) => c.clone(),
        None => return,
    };

    if !is_hidden(win) {
        return;
    }

    let x = client.geo.x;
    let y = client.geo.y;
    let w = client.geo.w;
    let h = client.geo.h;

    drop(globals);

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.map_window(win);
        let _ = conn.flush();
    }

    set_client_state(win, WM_STATE_NORMAL);
    resize(win, x, -50, w, h, false);

    if let Some(ref conn) = x11.conn {
        let _ = conn.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        let _ = conn.flush();
    }

    animate_client(win, x, y, 0, 0, 14, 0);

    let mon_id = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            c.mon_id
        } else {
            None
        }
    };
    if let Some(mid) = mon_id {
        arrange(Some(mid));
    }
}

pub fn hide(win: Window) {
    let globals = get_globals();
    let client = match globals.clients.get(&win) {
        Some(c) => c.clone(),
        None => return,
    };

    if is_hidden(win) {
        return;
    }

    let x = client.geo.x;
    let y = client.geo.y;
    let w = client.geo.w;
    let h = client.geo.h;
    let mon_id = client.mon_id;
    let bh = globals.bh;
    let animated = globals.animated;

    drop(globals);

    if animated {
        animate_client(win, x, bh - h + 40, 0, 0, 10, 0);
    }

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();

        let _ = conn.grab_server();

        let root_attrs = conn.get_window_attributes(globals.root);
        let win_attrs = conn.get_window_attributes(win);

        if let (Ok(root_cookie), Ok(win_cookie)) = (root_attrs, win_attrs) {
            if let (Ok(root_ra), Ok(win_ca)) = (root_cookie.reply(), win_cookie.reply()) {
                let root_mask = EventMask::from(
                    root_ra.your_event_mask.bits() & !EventMask::SUBSTRUCTURE_NOTIFY.bits(),
                );
                let win_mask = EventMask::from(
                    win_ca.your_event_mask.bits() & !EventMask::STRUCTURE_NOTIFY.bits(),
                );

                let _ = conn.change_window_attributes(
                    globals.root,
                    &ChangeWindowAttributesAux::new().event_mask(root_mask),
                );
                let _ = conn.change_window_attributes(
                    win,
                    &ChangeWindowAttributesAux::new().event_mask(win_mask),
                );
            }
        }

        let _ = conn.unmap_window(win);
        let _ = conn.flush();

        set_client_state(win, WM_STATE_ICONIC);

        let root_attrs = conn.get_window_attributes(globals.root);
        let win_attrs = conn.get_window_attributes(win);
        if let (Ok(root_cookie), Ok(win_cookie)) = (root_attrs, win_attrs) {
            if let Ok(root_ra) = root_cookie.reply() {
                let _ = conn.change_window_attributes(
                    globals.root,
                    &ChangeWindowAttributesAux::new().event_mask(root_ra.your_event_mask),
                );
            }
            if let Ok(win_ca) = win_cookie.reply() {
                let _ = conn.change_window_attributes(
                    win,
                    &ChangeWindowAttributesAux::new().event_mask(win_ca.your_event_mask),
                );
            }
        }

        let _ = conn.ungrab_server();
        let _ = conn.flush();
    }

    resize(win, x, y, w, h, false);

    let snext = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            c.snext
        } else {
            None
        }
    };
    crate::focus::focus(snext);

    if let Some(mid) = mon_id {
        arrange(Some(mid));
    }
}

pub fn resize(win: Window, x: i32, y: i32, w: i32, h: i32, interact: bool) {
    let mut globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        let mut nx = x;
        let mut ny = y;
        let mut nw = w;
        let mut nh = h;
        let result = apply_size_hints(client, &mut nx, &mut ny, &mut nw, &mut nh, interact);
        let client_count = globals.clients.len();
        if result || client_count == 1 {
            drop(globals);
            resize_client(win, nx, ny, nw, nh);
        }
    }
}

pub fn resize_client(win: Window, x: i32, y: i32, w: i32, h: i32) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.old_geo.x = client.geo.x;
            client.geo.x = x;
            client.old_geo.y = client.geo.y;
            client.geo.y = y;
            client.old_geo.w = client.geo.w;
            client.geo.w = w;
            client.old_geo.h = client.geo.h;
            client.geo.h = h;

            let border_width = client.border_width;

            let _ = conn.configure_window(
                win,
                &ConfigureWindowAux::new()
                    .x(x)
                    .y(y)
                    .width(w as u32)
                    .height(h as u32)
                    .border_width(border_width as u32),
            );
        }
        drop(globals);

        configure(win);
        let _ = conn.flush();
    }
}

pub fn update_title(win: Window) {
    let name = read_window_title(win);
    let mut globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        client.name = name;
    }
}

fn read_window_title(win: Window) -> String {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else {
        return BROKEN.to_string();
    };

    let net_wm_name = get_globals().netatom.wm_name;
    for atom in [net_wm_name, AtomEnum::WM_NAME.into()] {
        if atom == 0 {
            continue;
        }

        let Ok(cookie) = conn.get_property(false, win, atom, AtomEnum::ANY, 0, 1024) else {
            continue;
        };
        let Ok(reply) = cookie.reply() else {
            continue;
        };

        if reply.format != 8 || reply.value.is_empty() {
            continue;
        }

        let len = reply
            .value
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(reply.value.len());
        let title = String::from_utf8_lossy(&reply.value[..len]).into_owned();
        if !title.is_empty() {
            return title;
        }
    }

    BROKEN.to_string()
}

pub fn apply_rules(win: Window) {
    let x11 = get_x11();
    let (class, instance) = if let Some(ref conn) = x11.conn {
        let hint = conn.get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024);

        if let Ok(cookie) = hint {
            if let Ok(reply) = cookie.reply() {
                let data: Vec<u8> = reply.value8().map(|v| v.collect()).unwrap_or_default();
                let parts: Vec<&[u8]> = data.split(|&b| b == 0).filter(|s| !s.is_empty()).collect();
                // WM_CLASS is encoded as: instance\0class\0
                let instance: Vec<u8> = parts
                    .get(0)
                    .map(|s| s.to_vec())
                    .unwrap_or_else(|| BROKEN.as_bytes().to_vec());
                let class: Vec<u8> = parts
                    .get(1)
                    .map(|s| s.to_vec())
                    .unwrap_or_else(|| BROKEN.as_bytes().to_vec());
                (class, instance)
            } else {
                (BROKEN.as_bytes().to_vec(), BROKEN.as_bytes().to_vec())
            }
        } else {
            (BROKEN.as_bytes().to_vec(), BROKEN.as_bytes().to_vec())
        }
    } else {
        return;
    };

    let mut globals = get_globals_mut();

    let special_next = globals.specialnext;
    let rules = globals.rules.clone();
    let tagmask = globals.tags.mask();
    let bh = globals.bh;

    if !globals.clients.contains_key(&win) {
        return;
    }

    // Read client info we need for matching
    let (client_name_copy, client_mon_id) = {
        let c = globals.clients.get(&win).unwrap();
        (c.name.clone(), c.mon_id)
    };

    // Initialize client fields
    if let Some(c) = globals.clients.get_mut(&win) {
        c.isfloating = false;
        c.tags = 0;
    }

    if special_next != SpecialNext::None {
        if let SpecialNext::Float = special_next {
            if let Some(c) = globals.clients.get_mut(&win) {
                c.isfloating = true;
            }
        }
        globals.specialnext = SpecialNext::None;
    } else {
        for rule in &rules {
            let title_match = rule
                .title
                .map(|t| {
                    let title_bytes = t.as_bytes();
                    client_name_copy
                        .as_bytes()
                        .windows(title_bytes.len())
                        .any(|w| w == title_bytes)
                })
                .unwrap_or(true);

            let class_match = rule
                .class
                .map(|c| class.windows(c.as_bytes().len()).any(|w| w == c.as_bytes()))
                .unwrap_or(true);

            let instance_match = rule
                .instance
                .map(|i| {
                    instance
                        .windows(i.as_bytes().len())
                        .any(|w| w == i.as_bytes())
                })
                .unwrap_or(true);

            if title_match && class_match && instance_match {
                if let Some(class_str) = rule.class {
                    if class_str == "Onboard" {
                        if let Some(c) = globals.clients.get_mut(&win) {
                            c.issticky = true;
                        }
                    }
                }

                let cur_mon_id = globals.clients.get(&win).and_then(|c| c.mon_id);
                let (mon_mw, mon_wh, mon_showbar, mon_my, mon_mx) = if let Some(mon_id) = cur_mon_id
                {
                    globals
                        .monitors
                        .get(mon_id)
                        .map(|m| {
                            (
                                m.monitor_rect.w,
                                m.work_rect.h,
                                m.showbar,
                                m.monitor_rect.y,
                                m.monitor_rect.x,
                            )
                        })
                        .unwrap_or((0, 0, false, 0, 0))
                } else {
                    (0, 0, false, 0, 0)
                };

                if let Some(c) = globals.clients.get_mut(&win) {
                    match rule.isfloating {
                        RuleFloat::FloatCenter => {
                            c.isfloating = true;
                        }
                        RuleFloat::FloatFullscreen => {
                            c.isfloating = true;
                            c.geo.w = mon_mw;
                            c.geo.h = mon_wh;
                            if mon_showbar {
                                c.geo.y = mon_my + bh;
                            }
                            c.geo.x = mon_mx;
                        }
                        RuleFloat::Scratchpad => {
                            c.isfloating = true;
                        }
                        RuleFloat::Float => {
                            c.isfloating = true;
                            if mon_showbar {
                                c.geo.y = mon_my + bh;
                            }
                        }
                        RuleFloat::Tiled => {
                            c.isfloating = false;
                        }
                    }

                    c.tags |= rule.tags;
                }

                let target_mon = globals.monitors.iter().position(|m| m.num == rule.monitor);
                if let Some(mid) = target_mon {
                    if let Some(c) = globals.clients.get_mut(&win) {
                        c.mon_id = Some(mid);
                    }
                }
            }
        }
    }

    let (client_mon_id, client_tags) = globals
        .clients
        .get(&win)
        .map(|c| (c.mon_id, c.tags))
        .unwrap_or((None, 0));
    if let Some(mid) = client_mon_id {
        let mon_tags = globals
            .monitors
            .get(mid)
            .map(|m| m.tagset[m.seltags as usize]);
        if let Some(mt) = mon_tags {
            if let Some(c) = globals.clients.get_mut(&win) {
                c.tags = if client_tags & tagmask != 0 {
                    client_tags & tagmask
                } else {
                    mt
                };
            }
        }
    }
}

pub fn apply_size_hints(
    c: &mut Client,
    x: &mut i32,
    y: &mut i32,
    w: &mut i32,
    h: &mut i32,
    interact: bool,
) -> bool {
    let globals = get_globals();

    *w = max(1, *w);
    *h = max(1, *h);

    if interact {
        if *x > globals.sw {
            *x = globals.sw - client_width(c);
        }
        if *y > globals.sh {
            *y = globals.sh - client_height(c);
        }
        if *x + *w + 2 * c.border_width < 0 {
            *x = 0;
        }
        if *y + *h + 2 * c.border_width < 0 {
            *y = 0;
        }
    } else if let Some(mon_id) = c.mon_id {
        if let Some(m) = globals.monitors.get(mon_id) {
            if *x >= m.work_rect.x + m.work_rect.w {
                *x = m.work_rect.x + m.work_rect.w - client_width(c);
            }
            if *y >= m.work_rect.y + m.work_rect.h {
                *y = m.work_rect.y + m.work_rect.h - client_height(c);
            }
            if *x + *w + 2 * c.border_width <= m.work_rect.x {
                *x = m.work_rect.x;
            }
            if *y + *h + 2 * c.border_width <= m.work_rect.y {
                *y = m.work_rect.y;
            }
        }
    }

    let bh = globals.bh;
    if *h < bh {
        *h = bh;
    }
    if *w < bh {
        *w = bh;
    }

    let resizehints = globals.resizehints;
    drop(globals);

    let has_arrange = {
        let globals = get_globals();
        if let Some(mon_id) = c.mon_id {
            if let Some(mon) = globals.monitors.get(mon_id) {
                mon.sellt == 0
            } else {
                true
            }
        } else {
            true
        }
    };

    if resizehints != 0 || c.isfloating || !has_arrange {
        if c.hintsvalid == 0 {
            update_size_hints(c);
        }

        let base_is_min = c.basew == c.minw && c.baseh == c.minh;

        if !base_is_min {
            *w -= c.basew;
            *h -= c.baseh;
        }

        if c.mina > 0.0 && c.maxa > 0.0 {
            if c.maxa < (*w as f32) / (*h as f32) {
                *w = (*h as f32 * c.maxa + 0.5) as i32;
            } else if c.mina < (*h as f32) / (*w as f32) {
                *h = (*w as f32 * c.mina + 0.5) as i32;
            }
        }

        if base_is_min {
            *w -= c.basew;
            *h -= c.baseh;
        }

        if c.incw != 0 {
            *w -= *w % c.incw;
        }
        if c.inch != 0 {
            *h -= *h % c.inch;
        }

        *w = max(*w + c.basew, c.minw);
        *h = max(*h + c.baseh, c.minh);

        if c.maxw != 0 {
            *w = min(*w, c.maxw);
        }
        if c.maxh != 0 {
            *h = min(*h, c.maxh);
        }
    }

    *x != c.geo.x || *y != c.geo.y || *w != c.geo.w || *h != c.geo.h
}

pub fn kill_client(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.sel
            } else {
                None
            }
        } else {
            None
        }
    };

    let Some(win) = sel_win else { return };

    let (is_locked, is_fullscreen, mon_mh) = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            let mh = if let Some(mon_id) = c.mon_id {
                if let Some(mon) = globals.monitors.get(mon_id) {
                    mon.monitor_rect.h
                } else {
                    0
                }
            } else {
                0
            };
            (c.islocked, c.is_fullscreen, mh)
        } else {
            return;
        }
    };

    if is_locked {
        return;
    }

    let globals = get_globals();
    let animated = globals.animated;
    let anim_client = ANIM_CLIENT.load(Ordering::Relaxed);

    if animated && win != anim_client && !is_fullscreen {
        ANIM_CLIENT.store(win, Ordering::Relaxed);
        animate_client(win, 0, mon_mh - 20, 0, 0, 10, 0);
    }

    let wmatom_delete = globals.wmatom.delete;

    drop(globals);

    if !send_event(
        win,
        wmatom_delete,
        0,
        wmatom_delete as i64,
        CURRENT_TIME as i64,
        0,
        0,
        0,
    ) {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = conn.grab_server();
            let _ = conn.kill_client(win);
            let _ = conn.flush();
            let _ = conn.ungrab_server();
        }
    }
}

pub fn shut_kill(arg: &Arg) {
    let has_clients = {
        let globals = get_globals();
        globals
            .selmon
            .and_then(|id| globals.monitors.get(id))
            .map_or(false, |m| m.clients.is_some())
    };

    if !has_clients {
        let shut_arg = Arg {
            v: Some(crate::config::CMD_INSTANTSHUTDOWN),
            ..Default::default()
        };
        crate::util::spawn(&shut_arg);
    } else {
        kill_client(arg);
    }
}

pub fn close_win(arg: &Arg) {
    let win = match arg.v {
        Some(ptr) => ptr as u32,
        None => return,
    };

    let (is_locked, mon_mh) = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            let mh = if let Some(mon_id) = c.mon_id {
                if let Some(mon) = globals.monitors.get(mon_id) {
                    mon.monitor_rect.h
                } else {
                    0
                }
            } else {
                0
            };
            (c.islocked, mh)
        } else {
            (true, 0)
        }
    };

    if is_locked {
        return;
    }

    animate_client(win, 0, mon_mh - 20, 0, 0, 10, 0);

    let globals = get_globals();
    let wmatom_delete = globals.wmatom.delete;
    drop(globals);

    if !send_event(
        win,
        wmatom_delete,
        0,
        wmatom_delete as i64,
        CURRENT_TIME as i64,
        0,
        0,
        0,
    ) {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = conn.grab_server();
            let _ = conn.kill_client(win);
            let _ = conn.flush();
            let _ = conn.ungrab_server();
        }
    }
}

pub fn manage(
    w: Window,
    wa_x: i32,
    wa_y: i32,
    wa_width: u32,
    wa_height: u32,
    wa_border_width: u32,
) {
    let mut c = Client::default();
    c.win = w;
    c.geo.x = wa_x;
    c.old_geo.x = wa_x;
    c.geo.y = wa_y;
    c.old_geo.y = wa_y;
    c.geo.w = wa_width as i32;
    c.old_geo.w = wa_width as i32;
    c.geo.h = wa_height as i32;
    c.old_geo.h = wa_height as i32;
    c.old_border_width = wa_border_width as i32;
    c.name = read_window_title(w);

    let trans = get_transient_for_hint(w);

    {
        let globals = get_globals();
        let trans_client = trans.and_then(|t| win_to_client(t));

        if let (Some(_trans_win), Some(tc_win)) = (trans, trans_client) {
            if let Some(tc) = globals.clients.get(&tc_win) {
                c.mon_id = tc.mon_id;
                c.tags = tc.tags;
            }
        } else {
            c.mon_id = globals.selmon;
        }
    }

    {
        let mut globals = get_globals_mut();
        globals.clients.insert(w, c.clone());
    }

    apply_rules(w);

    let mut globals = get_globals_mut();
    let borderpx = globals.borderpx;
    if let Some(client) = globals.clients.get_mut(&w) {
        client.border_width = borderpx;
    }

    let (mon_mw, mon_mh, mon_mx, mon_my, mon_showbar, mon_ww, mon_wh, mon_wx, mon_wy) = {
        if let Some(mon_id) = c.mon_id {
            if let Some(mon) = globals.monitors.get(mon_id) {
                (
                    mon.monitor_rect.w,
                    mon.monitor_rect.h,
                    mon.monitor_rect.x,
                    mon.monitor_rect.y,
                    mon.showbar,
                    mon.work_rect.w,
                    mon.work_rect.h,
                    mon.work_rect.x,
                    mon.work_rect.y,
                )
            } else {
                (0, 0, 0, 0, false, 0, 0, 0, 0)
            }
        } else {
            (0, 0, 0, 0, false, 0, 0, 0, 0)
        }
    };

    if let Some(client) = globals.clients.get_mut(&w) {
        if client.geo.x + client_width(client) > mon_wx + mon_ww {
            client.geo.x = mon_wx + mon_ww - client_width(client);
        }
        if client.geo.y + client_height(client) > mon_wy + mon_wh {
            client.geo.y = mon_wy + mon_wh - client_height(client);
        }
        client.geo.x = max(client.geo.x, mon_wx);
        client.geo.y = max(client.geo.y, mon_wy);
    }

    let is_monocle = if let Some(mon_id) = c.mon_id {
        if let Some(mon) = globals.monitors.get(mon_id) {
            mon.sellt == 1
        } else {
            false
        }
    } else {
        false
    };

    let bh = globals.bh;

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let (isfloating, cw, ch) = if let Some(client) = globals.clients.get(&w) {
            (client.isfloating, client.geo.w, client.geo.h)
        } else {
            (false, 0, 0)
        };

        let border_width = if !isfloating && is_monocle && cw > mon_mw - 30 && ch > mon_mh - 30 - bh
        {
            0
        } else {
            borderpx
        };

        if let Some(client) = globals.clients.get_mut(&w) {
            client.border_width = border_width;
        }

        let _ = conn.configure_window(
            w,
            &ConfigureWindowAux::new().border_width(border_width as u32),
        );

        if let Some(ref scheme) = globals.borderscheme {
            let clr = &scheme.normal.bg;
            let _ = conn.change_window_attributes(
                w,
                &ChangeWindowAttributesAux::new().border_pixel(Some(clr.pixel() as u32)),
            );
        }
        let _ = conn.flush();
    }

    drop(globals);

    configure(w);
    update_window_type(w);
    update_size_hints_win(w);
    update_wm_hints(w);

    read_client_info(w);

    set_client_tag_prop(w);
    update_motif_hints(w);

    {
        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&w) {
            client.float_geo.x = client.geo.x;
            client.float_geo.y = if client.geo.y >= mon_my {
                client.geo.y
            } else {
                client.geo.y + mon_my
            };
            client.float_geo.w = client.geo.w;
            client.float_geo.h = client.geo.h;
        }
    }

    if let Some(ref conn) = x11.conn {
        let mask = EventMask::ENTER_WINDOW
            | EventMask::FOCUS_CHANGE
            | EventMask::PROPERTY_CHANGE
            | EventMask::STRUCTURE_NOTIFY;
        let _ =
            conn.change_window_attributes(w, &ChangeWindowAttributesAux::new().event_mask(mask));
    }

    grab_buttons(w, false);

    let isfixed = {
        let globals = get_globals();
        globals
            .clients
            .get(&w)
            .map(|client| client.isfixed)
            .unwrap_or(false)
    };

    let mut should_raise = false;
    {
        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&w) {
            if !client.isfloating {
                client.isfloating = trans.is_some() || isfixed;
                client.oldstate = client.isfloating as i32;
            }
            should_raise = client.isfloating;
        }
    }
    if should_raise {
        if let Some(ref conn) = x11.conn {
            let _ =
                conn.configure_window(w, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
            let _ = conn.flush();
        }
    }

    attach(w);
    attach_stack(w);

    {
        let globals = get_globals();
        if let Some(ref conn) = x11.conn {
            let _ = conn.change_property32(
                PropMode::APPEND,
                globals.root,
                globals.netatom.client_list,
                AtomEnum::WINDOW,
                &[w],
            );
            let _ = conn.flush();
        }
    }

    let (sw, cx, cy, cw, ch) = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&w) {
            (
                globals.sw,
                client.geo.x,
                client.geo.y,
                client.geo.w,
                client.geo.h,
            )
        } else {
            return;
        }
    };

    if let Some(ref conn) = x11.conn {
        let _ = conn.configure_window(
            w,
            &ConfigureWindowAux::new()
                .x(cx + 2 * sw)
                .y(cy)
                .width(cw as u32)
                .height(ch as u32),
        );
        let _ = conn.flush();
    }

    if !is_hidden(w) {
        set_client_state(w, WM_STATE_NORMAL);
    }

    let sel_win = {
        let globals = get_globals();
        globals
            .selmon
            .and_then(|sel_mon_id| globals.monitors.get(sel_mon_id))
            .and_then(|mon| mon.sel)
    };
    if let Some(sel_win) = sel_win {
        unfocus_win(sel_win, false);
    }

    let animated = {
        let mut globals = get_globals_mut();
        if let Some(mon_id) = c.mon_id {
            if let Some(mon) = globals.monitors.get_mut(mon_id) {
                mon.sel = Some(w);
            }
        }
        globals.animated
    };

    if let Some(mon_id) = c.mon_id {
        arrange(Some(mon_id));
    }

    if !is_hidden(w) {
        if let Some(ref conn) = x11.conn {
            let _ = conn.map_window(w);
            let _ = conn.flush();
        }
    }

    crate::focus::focus(None);

    if animated && !c.is_fullscreen {
        resize_client(w, c.geo.x, c.geo.y - 70, c.geo.w, c.geo.h);
        animate_client(w, c.geo.x, c.geo.y + 70, 0, 0, 7, 0);

        let has_arrange = if let Some(mon_id) = c.mon_id {
            let globals = get_globals();
            if let Some(mon) = globals.monitors.get(mon_id) {
                mon.sellt == 0
            } else {
                false
            }
        } else {
            false
        };

        if !has_arrange {
            if let Some(ref conn) = x11.conn {
                let _ = conn
                    .configure_window(w, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
                let _ = conn.flush();
            }
        } else if c.geo.w > mon_mw - 30 || c.geo.h > mon_mh - 30 {
            if let Some(mon_id) = c.mon_id {
                arrange(Some(mon_id));
            }
        }
    }
}

fn get_transient_for_hint(w: Window) -> Option<Window> {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        if let Ok(cookie) =
            conn.get_property(false, w, AtomEnum::WM_TRANSIENT_FOR, AtomEnum::WINDOW, 0, 1)
        {
            if let Ok(reply) = cookie.reply() {
                return reply.value32().and_then(|mut v| v.next());
            }
        }
    }
    None
}

fn read_client_info(w: Window) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let client_info_atom = {
            let globals = get_globals();
            globals.netatom.client_info
        };
        if let Ok(cookie) = conn.get_property(false, w, client_info_atom, AtomEnum::CARDINAL, 0, 2)
        {
            if let Ok(reply) = cookie.reply() {
                if let Some(mut data) = reply.value32() {
                    let tags = data.next().unwrap_or(0);
                    let mon_num = data.next().unwrap_or(0);

                    let target_mon = {
                        let globals = get_globals();
                        globals
                            .monitors
                            .iter()
                            .position(|m| m.num as u32 == mon_num)
                    };
                    let mut globals = get_globals_mut();
                    if let Some(client) = globals.clients.get_mut(&w) {
                        client.tags = tags;
                        if let Some(mid) = target_mon {
                            client.mon_id = Some(mid);
                        }
                    }
                }
            }
        }
    }
}

pub fn unmanage(win: Window, destroyed: bool) {
    let mon_id = {
        let globals = get_globals();
        globals.clients.get(&win).and_then(|c| c.mon_id)
    };

    let is_overlay = {
        let globals = get_globals();
        globals.monitors.iter().any(|m| m.overlay == Some(win))
    };

    if is_overlay {
        let mut globals = get_globals_mut();
        for mon in &mut globals.monitors {
            if mon.overlay == Some(win) {
                mon.overlay = None;
            }
        }
    }

    detach(win);
    detach_stack(win);

    if !destroyed {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let old_bw = {
                let globals = get_globals();
                globals
                    .clients
                    .get(&win)
                    .map(|c| c.old_border_width)
                    .unwrap_or(0)
            };

            let _ = conn.grab_server();

            let _ = conn.change_window_attributes(
                win,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
            );
            let _ =
                conn.configure_window(win, &ConfigureWindowAux::new().border_width(old_bw as u32));
            let _ = ungrab_button(conn, ButtonIndex::from(0u8), win, ModMask::from(0u16));

            set_client_state(win, WM_STATE_WITHDRAWN);

            let _ = conn.flush();
            let _ = conn.ungrab_server();
        }
    }

    let mut globals = get_globals_mut();
    globals.clients.remove(&win);

    drop(globals);
    crate::focus::focus(None);
    update_client_list();

    if let Some(mid) = mon_id {
        arrange(Some(mid));
    }
}

pub fn set_fullscreen(win: Window, fullscreen: bool) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let (net_wm_fullscreen, net_wm_state) = {
            let globals = get_globals();
            (globals.netatom.wm_fullscreen, globals.netatom.wm_state)
        };

        let mut globals = get_globals_mut();

        // Read client state first to avoid borrow conflicts
        let client_state = globals.clients.get(&win).map(|c| {
            (
                c.is_fullscreen,
                c.isfloating,
                c.isfakefullscreen,
                c.mon_id,
                c.oldstate,
                c.old_geo.x,
                c.old_geo.y,
                c.old_geo.w,
                c.old_geo.h,
            )
        });
        let Some((is_fs, is_floating, is_fake_fs, mon_id, _oldstate, oldx, oldy, oldw, oldh)) =
            client_state
        else {
            return;
        };

        if fullscreen && !is_fs {
            let _ = conn.change_property32(
                PropMode::REPLACE,
                win,
                net_wm_state,
                AtomEnum::ATOM,
                &[net_wm_fullscreen],
            );

            if let Some(c) = globals.clients.get_mut(&win) {
                c.is_fullscreen = true;
                c.oldstate = c.isfloating as i32;
            }
            save_bw(win);

            if !is_fake_fs {
                if let Some(c) = globals.clients.get_mut(&win) {
                    c.border_width = 0;
                }
                let (mon_mx, mon_my, mon_mw, mon_mh) = if let Some(mid) = mon_id {
                    globals
                        .monitors
                        .get(mid)
                        .map(|m| {
                            (
                                m.monitor_rect.x,
                                m.monitor_rect.y,
                                m.monitor_rect.w,
                                m.monitor_rect.h,
                            )
                        })
                        .unwrap_or((0, 0, 0, 0))
                } else {
                    (0, 0, 0, 0)
                };

                if !is_floating {
                    drop(globals);
                    animate_client(win, mon_mx, mon_my, mon_mw, mon_mh, 10, 0);
                    globals = get_globals_mut();
                }

                let _ = conn.configure_window(
                    win,
                    &ConfigureWindowAux::new()
                        .x(mon_mx)
                        .y(mon_my)
                        .width(mon_mw as u32)
                        .height(mon_mh as u32),
                );
                let _ = conn
                    .configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
                let _ = conn.flush();
            }

            if let Some(c) = globals.clients.get_mut(&win) {
                c.isfloating = true;
            }
        } else if !fullscreen && is_fs {
            let _ =
                conn.change_property32(PropMode::REPLACE, win, net_wm_state, AtomEnum::ATOM, &[]);

            if let Some(c) = globals.clients.get_mut(&win) {
                c.is_fullscreen = false;
                c.isfloating = c.oldstate != 0;
            }
            restore_border_width(win);

            if !is_fake_fs {
                drop(globals);
                resize_client(win, oldx, oldy, oldw, oldh);
                if let Some(mid) = mon_id {
                    arrange(Some(mid));
                }
            }
        }
    }
}

pub fn toggle_fake_fullscreen(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.sel
            } else {
                None
            }
        } else {
            None
        }
    };

    let Some(win) = sel_win else { return };

    let (is_fullscreen, isfakefullscreen, mon_id, old_border_width) = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| {
                (
                    c.is_fullscreen,
                    c.isfakefullscreen,
                    c.mon_id,
                    c.old_border_width,
                )
            })
            .unwrap_or((false, false, None, 0))
    };

    if is_fullscreen && isfakefullscreen {
        let borderpx = get_globals().borderpx;
        if let Some(mid) = mon_id {
            let (mon_mx, mon_my, mon_mw, mon_mh) = get_globals()
                .monitors
                .get(mid)
                .map(|m| {
                    (
                        m.monitor_rect.x,
                        m.monitor_rect.y,
                        m.monitor_rect.w,
                        m.monitor_rect.h,
                    )
                })
                .unwrap_or((0, 0, 0, 0));
            resize_client(
                win,
                mon_mx + borderpx,
                mon_my + borderpx,
                mon_mw - 2 * borderpx,
                mon_mh - 2 * borderpx,
            );

            let x11 = get_x11();
            if let Some(ref conn) = x11.conn {
                let _ = conn
                    .configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
                let _ = conn.flush();
            }
        }
    }

    let mut globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        if client.is_fullscreen {
            if !client.isfakefullscreen {
                client.border_width = old_border_width;
            }
        }

        client.isfakefullscreen = !client.isfakefullscreen;
    }
}

pub fn update_size_hints(c: &mut Client) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        if let Ok(cookie) = conn.get_property(
            false,
            c.win,
            AtomEnum::WM_NORMAL_HINTS,
            AtomEnum::WM_SIZE_HINTS,
            0,
            24,
        ) {
            if let Ok(reply) = cookie.reply() {
                let data = reply
                    .value8()
                    .map(|v| v.collect::<Vec<u8>>())
                    .unwrap_or_default();

                let flags = if data.len() >= 4 {
                    u32::from_ne_bytes([data[0], data[1], data[2], data[3]])
                } else {
                    0
                };

                const P_BASE_SIZE: u32 = 8;
                const P_MIN_SIZE: u32 = 16;
                const P_MAX_SIZE: u32 = 32;
                const P_RESIZE_INC: u32 = 64;
                const P_ASPECT: u32 = 128;

                if flags & P_BASE_SIZE != 0 && data.len() >= 28 {
                    c.basew = i32::from_ne_bytes([data[8], data[9], data[10], data[11]]);
                    c.baseh = i32::from_ne_bytes([data[12], data[13], data[14], data[15]]);
                } else if flags & P_MIN_SIZE != 0 && data.len() >= 28 {
                    c.basew = i32::from_ne_bytes([data[16], data[17], data[18], data[19]]);
                    c.baseh = i32::from_ne_bytes([data[20], data[21], data[22], data[23]]);
                } else {
                    c.basew = 0;
                    c.baseh = 0;
                }

                if flags & P_RESIZE_INC != 0 && data.len() >= 36 {
                    c.incw = i32::from_ne_bytes([data[24], data[25], data[26], data[27]]);
                    c.inch = i32::from_ne_bytes([data[28], data[29], data[30], data[31]]);
                } else {
                    c.incw = 0;
                    c.inch = 0;
                }

                if flags & P_MAX_SIZE != 0 && data.len() >= 44 {
                    c.maxw = i32::from_ne_bytes([data[32], data[33], data[34], data[35]]);
                    c.maxh = i32::from_ne_bytes([data[36], data[37], data[38], data[39]]);
                } else {
                    c.maxw = 0;
                    c.maxh = 0;
                }

                if flags & P_MIN_SIZE != 0 && data.len() >= 52 {
                    c.minw = i32::from_ne_bytes([data[16], data[17], data[18], data[19]]);
                    c.minh = i32::from_ne_bytes([data[20], data[21], data[22], data[23]]);
                } else if flags & P_BASE_SIZE != 0 && data.len() >= 28 {
                    c.minw = c.basew;
                    c.minh = c.baseh;
                } else {
                    c.minw = 0;
                    c.minh = 0;
                }

                if flags & P_ASPECT != 0 && data.len() >= 72 {
                    let min_aspect_y = i32::from_ne_bytes([data[48], data[49], data[50], data[51]]);
                    let min_aspect_x = i32::from_ne_bytes([data[52], data[53], data[54], data[55]]);
                    let max_aspect_x = i32::from_ne_bytes([data[56], data[57], data[58], data[59]]);
                    let max_aspect_y = i32::from_ne_bytes([data[60], data[61], data[62], data[63]]);

                    c.mina = if min_aspect_x != 0 {
                        min_aspect_y as f32 / min_aspect_x as f32
                    } else {
                        0.0
                    };
                    c.maxa = if max_aspect_y != 0 {
                        max_aspect_x as f32 / max_aspect_y as f32
                    } else {
                        0.0
                    };
                } else {
                    c.maxa = 0.0;
                    c.mina = 0.0;
                }

                c.isfixed = c.maxw != 0 && c.maxh != 0 && c.maxw == c.minw && c.maxh == c.minh;
                c.hintsvalid = 1;
            }
        }
    }
}

fn update_size_hints_win(win: Window) {
    let mut globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        update_size_hints(client);
    }
}

pub fn update_window_type(win: Window) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();

        let state = get_atom_prop(conn, win, globals.netatom.wm_state);
        let wtype = get_atom_prop(conn, win, globals.netatom.wm_window_type);

        let atom_fullscreen = globals.netatom.wm_fullscreen;
        let atom_dialog = globals.netatom.wm_window_type_dialog;

        drop(globals);

        if state == Some(atom_fullscreen) {
            set_fullscreen(win, true);
        }

        if wtype == Some(atom_dialog) {
            let mut globals = get_globals_mut();
            if let Some(client) = globals.clients.get_mut(&win) {
                client.isfloating = true;
            }
        }
    }
}

fn get_atom_prop(
    conn: &x11rb::rust_connection::RustConnection,
    win: Window,
    atom: u32,
) -> Option<u32> {
    if let Ok(cookie) = conn.get_property(false, win, atom, AtomEnum::ATOM, 0, 1) {
        if let Ok(reply) = cookie.reply() {
            return reply.value32().and_then(|mut v| v.next());
        }
    }
    None
}

pub fn update_wm_hints(win: Window) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        if let Ok(cookie) =
            conn.get_property(false, win, AtomEnum::WM_HINTS, AtomEnum::WM_HINTS, 0, 9)
        {
            if let Ok(reply) = cookie.reply() {
                let data = reply
                    .value32()
                    .map(|v| v.collect::<Vec<u32>>())
                    .unwrap_or_default();

                let flags = if let Some(flags) = data.first().copied() {
                    flags
                } else {
                    return;
                };

                const INPUT_HINT: u32 = 1;
                const X_URGENCY_HINT: u32 = 256;

                let input = if flags & INPUT_HINT != 0 {
                    data.get(1).copied().unwrap_or(0) as i32
                } else {
                    0
                };

                let is_urgent = flags & X_URGENCY_HINT != 0;

                let mut globals = get_globals_mut();
                if let Some(client) = globals.clients.get_mut(&win) {
                    let is_sel = if let Some(sel_mon_id) = globals.selmon {
                        if let Some(mon) = globals.monitors.get(sel_mon_id) {
                            mon.sel == Some(win)
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if is_sel && is_urgent {
                        let new_flags = flags & !X_URGENCY_HINT;
                        let mut new_data = data.clone();
                        if new_data.is_empty() {
                            new_data.push(new_flags);
                        } else {
                            new_data[0] = new_flags;
                        }

                        drop(globals);
                        let _ = conn.change_property32(
                            PropMode::REPLACE,
                            win,
                            AtomEnum::WM_HINTS,
                            AtomEnum::WM_HINTS,
                            &new_data,
                        );
                        let _ = conn.flush();
                        globals = get_globals_mut();
                    }

                    if let Some(client) = globals.clients.get_mut(&win) {
                        client.isurgent = is_urgent;
                    }

                    if let Some(client) = globals.clients.get_mut(&win) {
                        if flags & INPUT_HINT != 0 {
                            client.neverfocus = input == 0;
                        } else {
                            client.neverfocus = false;
                        }
                    }
                }
            }
        }
    }
}

pub fn update_motif_hints(win: Window) {
    let globals = get_globals();
    if globals.decorhints == 0 {
        return;
    }
    let motif_atom = globals.motifatom;
    let borderpx = globals.borderpx;
    drop(globals);

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        if let Ok(cookie) = conn.get_property(false, win, motif_atom, motif_atom, 0, 5) {
            if let Ok(reply) = cookie.reply() {
                let data = reply
                    .value8()
                    .map(|v| v.collect::<Vec<u8>>())
                    .unwrap_or_default();
                if data.len() >= 20 {
                    let motif: Vec<u32> = data
                        .chunks_exact(4)
                        .map(|chunk| u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                        .collect();

                    if motif.len() > MWM_HINTS_FLAGS_FIELD
                        && (motif[MWM_HINTS_FLAGS_FIELD] & MWM_HINTS_DECORATIONS) != 0
                    {
                        let (c_w, c_h, c_x, c_y) = {
                            let globals = get_globals();
                            if let Some(c) = globals.clients.get(&win) {
                                (client_width(c), client_height(c), c.geo.x, c.geo.y)
                            } else {
                                return;
                            }
                        };

                        let decorations =
                            motif.get(MWM_HINTS_DECORATIONS_FIELD).copied().unwrap_or(0);
                        let new_bw = if (decorations & MWM_DECOR_ALL) != 0
                            || (decorations & MWM_DECOR_BORDER) != 0
                            || (decorations & MWM_DECOR_TITLE) != 0
                        {
                            borderpx
                        } else {
                            0
                        };

                        {
                            let mut globals = get_globals_mut();
                            if let Some(client) = globals.clients.get_mut(&win) {
                                client.border_width = new_bw;
                                client.old_border_width = new_bw;
                            }
                        }

                        resize(win, c_x, c_y, c_w - 2 * new_bw, c_h - 2 * new_bw, false);
                    }
                }
            }
        }
    }
}

pub fn is_hidden(win: Window) -> bool {
    get_state(win) == WM_STATE_ICONIC
}

fn grab_buttons(win: Window, focused: bool) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = ungrab_button(conn, ButtonIndex::from(0u8), win, ModMask::from(0u16));

        if !focused {
            let globals = get_globals();
            let numlockmask = globals.numlockmask;

            let button_mask: u32 =
                EventMask::BUTTON_PRESS.bits() | EventMask::BUTTON_RELEASE.bits();
            let lock_mask: u32 = x11rb::protocol::xproto::ModMask::LOCK.bits() as u32;

            for &modifiers in &[0, numlockmask, lock_mask, numlockmask | lock_mask] {
                let _ = conn.grab_button(
                    false,
                    win,
                    button_mask.into(),
                    GrabMode::SYNC,
                    GrabMode::SYNC,
                    0u32,
                    0u32,
                    1u8.into(),
                    ModMask::from(modifiers as u16),
                );
                let _ = conn.grab_button(
                    false,
                    win,
                    button_mask.into(),
                    GrabMode::SYNC,
                    GrabMode::SYNC,
                    0u32,
                    0u32,
                    3u8.into(),
                    ModMask::from(modifiers as u16),
                );
            }
        }
        let _ = conn.flush();
    }
}

pub fn save_bw(win: Window) {
    let mut globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        client.old_border_width = client.border_width;
    }
}

pub fn restore_border_width(win: Window) {
    let mut globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        client.border_width = client.old_border_width;
    }
}

pub fn update_client_list() {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let _ = conn.delete_property(globals.root, globals.netatom.client_list);

        for mon in &globals.monitors {
            let mut current = mon.clients;
            while let Some(cur_win) = current {
                let _ = conn.change_property32(
                    PropMode::APPEND,
                    globals.root,
                    globals.netatom.client_list,
                    AtomEnum::WINDOW,
                    &[cur_win],
                );
                current = if let Some(c) = globals.clients.get(&cur_win) {
                    c.next
                } else {
                    None
                };
            }
        }
        let _ = conn.flush();
    }
}

pub fn zoom(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.sel
            } else {
                None
            }
        } else {
            None
        }
    };

    let Some(win) = sel_win else { return };

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        let _ = conn.flush();
    }

    let (is_floating, mon_id) = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            (c.isfloating, c.mon_id)
        } else {
            (true, None)
        }
    };

    let has_arrange = if let Some(mid) = mon_id {
        let globals = get_globals();
        if let Some(mon) = globals.monitors.get(mid) {
            mon.sellt == 0
        } else {
            true
        }
    } else {
        true
    };

    if !has_arrange || is_floating {
        return;
    }

    let first_tiled = {
        let globals = get_globals();
        if let Some(mid) = mon_id {
            if let Some(mon) = globals.monitors.get(mid) {
                next_tiled(mon.clients)
            } else {
                None
            }
        } else {
            None
        }
    };

    if first_tiled.map_or(false, |t| win == t) {
        let globals = get_globals();
        let next = if let Some(f) = first_tiled {
            if let Some(c) = globals.clients.get(&f) {
                next_tiled(c.next)
            } else {
                None
            }
        } else {
            None
        };
        if next.is_none() {
            return;
        }
    }

    pop(win);
}

pub fn set_urgent(win: Window, urg: bool) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.isurgent = urg;
        }

        if let Ok(cookie) =
            conn.get_property(false, win, AtomEnum::WM_HINTS, AtomEnum::WM_HINTS, 0, 9)
        {
            if let Ok(reply) = cookie.reply() {
                let data = reply
                    .value8()
                    .map(|v| v.collect::<Vec<u8>>())
                    .unwrap_or_default();
                let flags = if data.len() >= 4 {
                    u32::from_ne_bytes([data[0], data[1], data[2], data[3]])
                } else {
                    return;
                };

                const X_URGENCY_HINT: u32 = 256;
                let new_flags = if urg {
                    flags | X_URGENCY_HINT
                } else {
                    flags & !X_URGENCY_HINT
                };

                let mut new_data = vec![0u8; data.len().max(36)];
                new_data[..4].copy_from_slice(&new_flags.to_ne_bytes());
                if data.len() > 4 {
                    new_data[4..data.len()].copy_from_slice(&data[4..]);
                }

                let _ = conn.change_property(
                    PropMode::REPLACE,
                    win,
                    AtomEnum::WM_HINTS,
                    AtomEnum::WM_HINTS,
                    8u8,
                    new_data.len() as u32,
                    &new_data,
                );
                let _ = conn.flush();
            }
        }
    }
}

pub fn scale_client(win: Window, scale: i32) {
    let mut globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        let mon_id = client.mon_id;
        let old_x = client.geo.x;
        let old_y = client.geo.y;
        let old_w = client.geo.w;
        let old_h = client.geo.h;
        let border_width = client.border_width;

        drop(globals);

        let (mon_mw, mon_mh, mon_mx, mon_my) = if let Some(mid) = mon_id {
            let globals = get_globals();
            if let Some(mon) = globals.monitors.get(mid) {
                (
                    mon.monitor_rect.w,
                    mon.monitor_rect.h,
                    mon.monitor_rect.x,
                    mon.monitor_rect.y,
                )
            } else {
                (old_w, old_h, old_x, old_y)
            }
        } else {
            (old_w, old_h, old_x, old_y)
        };

        let new_w = old_w * scale / 100;
        let new_h = old_h * scale / 100;
        let new_x = mon_mx + (mon_mw - new_w) / 2 - border_width;
        let new_y = mon_my + (mon_mh - new_h) / 2 - border_width;

        resize(win, new_x, new_y, new_w, new_h, false);
    }
}

pub fn save_floating(win: Window) {
    let mut globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        client.float_geo.x = client.geo.x;
        client.float_geo.y = client.geo.y;
        client.float_geo.w = client.geo.w;
        client.float_geo.h = client.geo.h;
    }
}

pub fn restore_floating(win: Window) {
    let (x, y, w, h) = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&win) {
            (
                client.float_geo.x,
                client.float_geo.y,
                client.float_geo.w,
                client.float_geo.h,
            )
        } else {
            return;
        }
    };
    resize(win, x, y, w, h, false);
}

pub fn change_floating(win: Window) {
    let mut globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        if client.snapstatus != SnapPosition::None {
            client.snapstatus = SnapPosition::None;
        }
    }
}
