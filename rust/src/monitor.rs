use crate::bar::update_bar_pos;
use crate::client::{
    attach, attach_stack, detach, detach_stack, is_visible, set_client_tag_prop, unfocus_win,
    win_to_client as get_win_to_client,
};
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::scratchpad::scratchpad_show;
use crate::types::*;
use x11rb::protocol::xproto::Window;

#[cfg(feature = "xinerama")]
use x11rb::protocol::xinerama;

fn tagmon(arg: &Arg) {
    if let Some(target) = dir_to_mon(arg.i) {
        let g = get_globals();
        if let Some(mon_id) = g.selmon {
            if let Some(client) = g.monitors.get(mon_id).and_then(|_m| {
                g.clients
                    .values()
                    .find(|c| c.mon_id == Some(mon_id))
                    .map(|c| c.win)
            }) {
                drop(g);
                send_mon(client, target);
            }
        }
    }
}

pub fn create_monitor() -> MonitorInner {
    let g = get_globals();

    let mut m = MonitorInner {
        tagset: [1, 1],
        mfact: g.mfact,
        nmaster: g.nmaster,
        showbar: g.showbar,
        topbar: g.topbar,
        clientcount: 0,
        overlaymode: 0,
        sellt: 0,
        ..Default::default()
    };

    let default_symbol = b"[]=";
    m.ltsymbol[..default_symbol.len()].copy_from_slice(default_symbol);

    let pertag = Box::new(Pertag {
        current_tag: 1,
        prevtag: 1,
        nmasters: [m.nmaster; MAX_TAGS],
        mfacts: [m.mfact; MAX_TAGS],
        sellts: [m.sellt; MAX_TAGS],
        showbars: [m.showbar; MAX_TAGS],
        ltidxs: [[None; 2]; MAX_TAGS],
    });

    m.pertag = Some(pertag);

    m
}

pub fn cleanup_monitor(mon_id: MonitorId) {
    let mut g = get_globals_mut();

    if mon_id >= g.monitors.len() {
        return;
    }

    let barwin = g.monitors[mon_id].barwin;

    g.monitors.remove(mon_id);

    if g.selmon == Some(mon_id) {
        g.selmon = if g.monitors.is_empty() { None } else { Some(0) };
    } else if let Some(sel) = g.selmon {
        if sel > mon_id {
            g.selmon = Some(sel - 1);
        }
    }

    drop(g);

    if barwin != 0 {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = x11rb::protocol::xproto::unmap_window(conn, barwin);
            let _ = x11rb::protocol::xproto::destroy_window(conn, barwin);
        }
    }
}

pub fn dir_to_mon(dir: i32) -> Option<MonitorId> {
    let g = get_globals();
    let selmon = g.selmon?;

    if g.monitors.len() <= 1 {
        return Some(selmon);
    }

    if dir > 0 {
        if selmon + 1 >= g.monitors.len() {
            Some(0)
        } else {
            Some(selmon + 1)
        }
    } else if selmon == 0 {
        Some(g.monitors.len() - 1)
    } else {
        Some(selmon - 1)
    }
}

pub fn rect_to_mon(x: i32, y: i32, w: i32, h: i32) -> Option<MonitorId> {
    let g = get_globals();

    let selmon = g.selmon?;
    let mut result = selmon;
    let mut max_area = 0;

    for (i, m) in g.monitors.iter().enumerate() {
        let area = intersect(x, y, w, h, m);
        if area > max_area {
            max_area = area;
            result = i;
        }
    }

    Some(result)
}

pub fn win_to_mon(w: Window) -> Option<MonitorId> {
    let g = get_globals();

    if w == g.root {
        if let Some((x, y)) = get_root_ptr() {
            return rect_to_mon(x, y, 1, 1);
        }
        return g.selmon;
    }

    for (i, m) in g.monitors.iter().enumerate() {
        if w == m.barwin {
            return Some(i);
        }
    }

    if let Some(win) = get_win_to_client(w) {
        let g = get_globals();
        return g.clients.get(&win).and_then(|c| c.mon_id);
    }

    g.selmon
}

pub fn send_mon(c_win: Window, target_mon_id: MonitorId) {
    let g = get_globals_mut();

    let current_mon_id = match g.selmon {
        Some(id) => id,
        None => return,
    };

    if current_mon_id == target_mon_id {
        return;
    }

    let (is_scratchpad, target_tags) = {
        let client = match g.clients.get(&c_win) {
            Some(c) => c,
            None => return,
        };
        let is_sp = client.tags == SCRATCHPAD_MASK;
        let tags = if !is_sp {
            g.monitors
                .get(target_mon_id)
                .map(|m| m.tagset[m.seltags as usize])
                .unwrap_or(1)
        } else {
            0
        };
        (is_sp, tags)
    };

    drop(g);

    if let Some(_win) = get_win_to_client(c_win) {
        unfocus_win(c_win, true);
    }

    detach(c_win);
    detach_stack(c_win);

    {
        let mut g = get_globals_mut();
        if let Some(client) = g.clients.get_mut(&c_win) {
            client.mon_id = Some(target_mon_id);

            if !is_scratchpad {
                client.tags = target_tags;
                reset_sticky(&mut client.clone());
            }
        }
    }

    attach(c_win);
    attach_stack(c_win);
    set_client_tag_prop(c_win);

    focus(None);

    {
        let g = get_globals();
        if let Some(ref c) = g.clients.get(&c_win) {
            if !c.isfloating {
                drop(g);
                arrange(None);
            }
        }
    }

    if is_scratchpad {
        let g = get_globals();
        if let Some(ref c) = g.clients.get(&c_win) {
            if c.is_scratchpad() && !c.issticky {
                drop(g);

                let mut g = get_globals_mut();
                if let Some(ref mut sel) = g.selmon {
                    if let Some(win) = get_selected_client_win(*sel) {
                        unfocus_win(win, false);
                    }
                }
                g.selmon = Some(target_mon_id);
                drop(g);

                let name_ptr = {
                    let g = get_globals();
                    g.clients.get(&c_win).map(|c| {
                        let mut name = [0u8; SCRATCHPAD_NAME_LEN];
                        name.copy_from_slice(&c.scratchpad_name);
                        name
                    })
                };

                if let Some(name) = name_ptr {
                    let arg = Arg {
                        v: Some(unsafe { std::mem::transmute::<*const u8, usize>(name.as_ptr()) }),
                        ..Default::default()
                    };
                    scratchpad_show(&arg);
                }

                let mut g = get_globals_mut();
                if let Some(ref mut sel) = g.selmon {
                    if let Some(win) = get_selected_client_win(*sel) {
                        unfocus_win(win, false);
                    }
                }
                g.selmon = Some(current_mon_id);
                drop(g);

                focus(None);
            }
        }
    }
}

pub fn focus_mon(arg: &Arg) {
    let g = get_globals();

    if g.monitors.len() <= 1 {
        return;
    }

    let target = match dir_to_mon(arg.i) {
        Some(id) => id,
        None => return,
    };

    if Some(target) == g.selmon {
        return;
    }

    let old_sel = g.selmon;
    drop(g);

    if let Some(old_id) = old_sel {
        if let Some(win) = get_selected_client_win(old_id) {
            unfocus_win(win, false);
        }
    }

    let mut g = get_globals_mut();
    g.selmon = Some(target);
    drop(g);

    focus(None);
}

pub fn focus_n_mon(arg: &Arg) {
    let g = get_globals();

    if g.monitors.len() <= 1 {
        return;
    }

    let mut target = 0;
    for i in 0..arg.i as usize {
        if i + 1 < g.monitors.len() {
            target = i + 1;
        } else {
            break;
        }
    }

    let old_sel = g.selmon;
    drop(g);

    if let Some(old_id) = old_sel {
        if let Some(win) = get_selected_client_win(old_id) {
            unfocus_win(win, false);
        }
    }

    let mut g = get_globals_mut();
    g.selmon = Some(target);
    drop(g);

    focus(None);
}

pub fn follow_mon(arg: &Arg) {
    let c_win = {
        let g = get_globals();
        match g.selmon {
            Some(mon_id) => g.monitors.get(mon_id).and_then(|m| m.sel),
            None => None,
        }
    };

    let c_win = match c_win {
        Some(w) => w,
        None => return,
    };

    tagmon(arg);

    {
        let mut g = get_globals_mut();
        if let Some(ref c) = g.clients.get(&c_win) {
            g.selmon = c.mon_id;
        }
        drop(g);
    }

    focus(Some(c_win));

    {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = x11rb::protocol::xproto::configure_window(
                conn,
                c_win,
                &x11rb::protocol::xproto::ConfigureWindowAux::new()
                    .stack_mode(x11rb::protocol::xproto::StackMode::ABOVE),
            );
        }

        warp_cursor_to_client(c_win);
    }
}

#[cfg(feature = "xinerama")]
fn is_unique_geom(unique: &[(i32, i32, i32, i32)], info: (i32, i32, i32, i32)) -> bool {
    for u in unique {
        if u.0 == info.0 && u.1 == info.1 && u.2 == info.2 && u.3 == info.3 {
            return false;
        }
    }
    true
}

pub fn update_geom() -> bool {
    let mut dirty = false;

    #[cfg(feature = "xinerama")]
    {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            if let Ok(is_active_cookie) = xinerama::is_active(conn) {
                if let Ok(is_active) = is_active_cookie.reply() {
                    if is_active.state != 0 {
                        if let Ok(screens_cookie) = xinerama::query_screens(conn) {
                            if let Ok(screens) = screens_cookie.reply() {
                                let screen_info: Vec<(i32, i32, i32, i32)> = screens
                                    .screen_info
                                    .iter()
                                    .map(|s| {
                                        (
                                            s.x_org as i32,
                                            s.y_org as i32,
                                            s.width as i32,
                                            s.height as i32,
                                        )
                                    })
                                    .collect();

                                let mut unique: Vec<(i32, i32, i32, i32)> = Vec::new();
                                for info in &screen_info {
                                    if is_unique_geom(&unique, *info) {
                                        unique.push(*info);
                                    }
                                }

                                let nn = unique.len();
                                let n = {
                                    let g = get_globals();
                                    g.monitors.len()
                                };

                                {
                                    let mut g = get_globals_mut();
                                    while g.monitors.len() < nn {
                                        g.monitors.push(create_monitor());
                                    }
                                }

                                for (i, info) in unique.iter().enumerate() {
                                    let mut g = get_globals_mut();
                                    if i >= n {
                                        dirty = true;
                                    }

                                    if let Some(ref mut m) = g.monitors.get_mut(i) {
                                        if i >= n
                                            || m.mx != info.0
                                            || m.my != info.1
                                            || m.mw != info.2
                                            || m.mh != info.3
                                        {
                                            dirty = true;
                                            m.num = i as i32;
                                            m.mx = info.0;
                                            m.my = info.1;
                                            m.mw = info.2;
                                            m.mh = info.3;
                                            m.wx = info.0;
                                            m.wy = info.1;
                                            m.ww = info.2;
                                            m.wh = info.3;
                                            drop(g);
                                            let mut g = get_globals_mut();
                                            if let Some(ref mut m) = g.monitors.get_mut(i) {
                                                update_bar_pos(m);
                                            }
                                        }
                                    }
                                }

                                for i in nn..n {
                                    let clients_to_move: Vec<Window> = {
                                        let g = get_globals();
                                        g.clients
                                            .values()
                                            .filter(|c| c.mon_id == Some(i))
                                            .map(|c| c.win)
                                            .collect()
                                    };

                                    for win in clients_to_move {
                                        dirty = true;
                                        detach(win);
                                        detach_stack(win);

                                        let mut g = get_globals_mut();
                                        if let Some(ref mut c) = g.clients.get_mut(&win) {
                                            c.mon_id = Some(0);
                                        }
                                        drop(g);

                                        attach(win);
                                        attach_stack(win);
                                    }

                                    let mut g = get_globals_mut();
                                    if g.selmon == Some(i) {
                                        g.selmon = Some(0);
                                    }
                                    drop(g);

                                    cleanup_monitor(i);
                                }

                                if dirty {
                                    let mut g = get_globals_mut();
                                    g.selmon = Some(0);
                                    drop(g);

                                    if let Some(m) = win_to_mon(x11.screen_num as u32) {
                                        let mut g = get_globals_mut();
                                        g.selmon = Some(m);
                                    }
                                }

                                return dirty;
                            }
                        }
                    }
                }
            }
        }
    }

    let g = get_globals();
    if g.monitors.is_empty() {
        drop(g);
        let mut g = get_globals_mut();
        g.monitors.push(create_monitor());
        dirty = true;
    } else {
        let sw = g.sw;
        let sh = g.sh;
        let needs_update = g
            .monitors
            .first()
            .map(|m| m.mw != sw || m.mh != sh)
            .unwrap_or(false);
        drop(g);

        if needs_update {
            dirty = true;
            let mut g = get_globals_mut();
            if let Some(ref mut m) = g.monitors.first_mut() {
                m.mw = sw;
                m.mh = sh;
                m.ww = sw;
                m.wh = sh;
                update_bar_pos(m);
            }
        }
    }

    dirty
}

pub fn arrange(_m: Option<MonitorId>) {}

pub fn arrange_mon(_m: &mut MonitorInner) {}

pub fn restack(_m: &mut MonitorInner) {}

pub fn tag_mon(arg: &Arg) {
    crate::tags::tag_mon(arg);
}

fn get_root_ptr() -> Option<(i32, i32)> {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let g = get_globals();
        if let Ok(cookie) = x11rb::protocol::xproto::query_pointer(conn, g.root) {
            if let Ok(reply) = cookie.reply() {
                return Some((reply.root_x as i32, reply.root_y as i32));
            }
        }
    }
    None
}

fn get_selected_client(mon_id: MonitorId) -> Option<ClientInner> {
    let g = get_globals();
    if let Some(mon) = g.monitors.get(mon_id) {
        if let Some(win) = mon.sel {
            return g.clients.get(&win).cloned();
        }
    }
    None
}

fn get_selected_client_win(mon_id: MonitorId) -> Option<Window> {
    let g = get_globals();
    g.monitors.get(mon_id).and_then(|m| m.sel)
}

fn reset_sticky(_c: &mut ClientInner) {}

fn warp_cursor_to_client(_win: Window) {}
