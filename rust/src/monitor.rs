use crate::bar::update_bar_pos;
use crate::client::{
    attach, attach_stack, detach, detach_stack, set_client_tag_prop, unfocus_win,
    win_to_client as get_win_to_client,
};
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::types::*;
use x11rb::protocol::xproto::Window;

#[cfg(feature = "xinerama")]
use x11rb::protocol::xinerama;

//TODO: this is a bad name. Document function, rename to something more descriptive
fn tagmon(arg: &Arg) {
    if let Some(target) = dir_to_mon(arg.i) {
        let g = get_globals();
        let mon_id = g.selmon;
        if let Some(client) = g.monitors.get(mon_id).and_then(|_m| {
            g.clients
                .values()
                .find(|c| c.mon_id == Some(mon_id))
                .map(|c| c.win)
        }) {
            send_mon(client, target);
        }
    }
}

pub fn create_monitor() -> Monitor {
    eprintln!(
        "TRACE: create_monitor - start (WARNING: this version calls get_globals, may deadlock)"
    );
    let g = get_globals();
    eprintln!("TRACE: create_monitor - after get_globals");

    let mut m = Monitor {
        tagset: [1, 1],
        mfact: g.mfact,
        nmaster: g.nmaster,
        showbar: g.showbar,
        topbar: g.topbar,
        clientcount: 0,
        overlaymode: 0,
        current_tag: 1,
        prev_tag: 1,
        ..Default::default()
    };
    eprintln!("TRACE: create_monitor - after creating Monitor");

    eprintln!("TRACE: create_monitor - returning");
    m
}

pub fn create_monitor_with_values(
    mfact: f32,
    nmaster: i32,
    showbar: bool,
    topbar: bool,
) -> Monitor {
    eprintln!("TRACE: create_monitor_with_values - start");

    let mut m = Monitor {
        tagset: [1, 1],
        mfact,
        nmaster,
        showbar,
        topbar,
        clientcount: 0,
        overlaymode: 0,
        current_tag: 1,
        prev_tag: 1,
        ..Default::default()
    };
    eprintln!("TRACE: create_monitor_with_values - after creating Monitor");

    eprintln!("TRACE: create_monitor_with_values - returning");
    m
}

pub fn cleanup_monitor(mon_id: MonitorId) {
    let g = get_globals_mut();

    if mon_id >= g.monitors.len() {
        return;
    }

    let barwin = g.monitors[mon_id].barwin;

    g.monitors.remove(mon_id);

    if g.selmon == mon_id {
        g.selmon = 0;
    } else if g.selmon > mon_id {
        g.selmon -= 1;
    }

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
    if g.monitors.is_empty() {
        return None;
    }
    let selmon = g.selmon;

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
    if g.monitors.is_empty() {
        return None;
    }
    let selmon = g.selmon;
    let mut result = selmon;
    let mut max_area = 0;

    for (i, m) in g.monitors.iter().enumerate() {
        let area = intersect(&Rect { x, y, w, h }, m);
        if area > max_area {
            max_area = area;
            result = i;
        }
    }

    Some(result)
}

/// Find the monitor that a Rect intersects with most.
pub fn rect_to_mon_rect(rect: &Rect) -> Option<MonitorId> {
    rect_to_mon(rect.x, rect.y, rect.w, rect.h)
}

pub fn win_to_mon(w: Window) -> Option<MonitorId> {
    let g = get_globals();

    if w == g.root {
        if let Some((x, y)) = get_root_ptr() {
            return rect_to_mon_rect(&Rect { x, y, w: 1, h: 1 });
        }
        return if g.monitors.is_empty() {
            None
        } else {
            Some(g.selmon)
        };
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

    if g.monitors.is_empty() {
        None
    } else {
        Some(g.selmon)
    }
}

pub fn send_mon(c_win: Window, target_mon_id: MonitorId) {
    let g = get_globals_mut();

    let current_mon_id = g.selmon;

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

    if let Some(_win) = get_win_to_client(c_win) {
        unfocus_win(c_win, true);
    }

    detach(c_win);
    detach_stack(c_win);

    {
        let g = get_globals_mut();
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
                arrange(None);
            }
        }
    }

    if is_scratchpad {
        let g = get_globals();
        if let Some(ref c) = g.clients.get(&c_win) {
            if c.is_scratchpad() && !c.issticky {
                {
                    let g = get_globals_mut();
                    let sel = g.selmon;
                    if let Some(win) = get_selected_client_win(sel) {
                        unfocus_win(win, false);
                    }
                    g.selmon = target_mon_id;
                }

                let sp_name = {
                    let g = get_globals();
                    g.clients.get(&c_win).map(|c| c.scratchpad_name.clone())
                };

                if let Some(name) = sp_name {
                    crate::scratchpad::scratchpad_show_name(&name);
                }

                {
                    let g = get_globals_mut();
                    let sel = g.selmon;
                    if let Some(win) = get_selected_client_win(sel) {
                        unfocus_win(win, false);
                    }
                    g.selmon = current_mon_id;
                }

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

    if target == g.selmon {
        return;
    }

    let old_id = g.selmon;
    if let Some(win) = get_selected_client_win(old_id) {
        unfocus_win(win, false);
    }

    let g = get_globals_mut();
    g.selmon = target;

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

    let old_id = g.selmon;
    if let Some(win) = get_selected_client_win(old_id) {
        unfocus_win(win, false);
    }

    let g = get_globals_mut();
    g.selmon = target;

    focus(None);
}

pub fn follow_mon(arg: &Arg) {
    let c_win = {
        let g = get_globals();
        g.monitors.get(g.selmon).and_then(|m| m.sel)
    };

    let c_win = match c_win {
        Some(w) => w,
        None => return,
    };

    tagmon(arg);

    {
        let g = get_globals_mut();
        if let Some(ref c) = g.clients.get(&c_win) {
            if let Some(mon_id) = c.mon_id {
                g.selmon = mon_id;
            }
        }
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

/// Check if a Rect is unique in a list of Rects (no duplicate geometry).
#[cfg(feature = "xinerama")]
fn is_unique_geom(unique: &[Rect], info: &Rect) -> bool {
    !unique
        .iter()
        .any(|u| u.x == info.x && u.y == info.y && u.w == info.w && u.h == info.h)
}

pub fn update_geom() -> bool {
    eprintln!("TRACE: update_geom - start");
    let mut dirty = false;

    #[cfg(feature = "xinerama")]
    {
        eprintln!("TRACE: update_geom - before get_x11");
        let x11 = get_x11();
        eprintln!("TRACE: update_geom - after get_x11");
        if let Some(ref conn) = x11.conn {
            eprintln!("TRACE: update_geom - before is_active");
            if let Ok(is_active_cookie) = xinerama::is_active(conn) {
                eprintln!("TRACE: update_geom - before reply");
                if let Ok(is_active) = is_active_cookie.reply() {
                    eprintln!("TRACE: update_geom - is_active.state = {}", is_active.state);
                    if is_active.state != 0 {
                        eprintln!("TRACE: update_geom - before query_screens");
                        if let Ok(screens_cookie) = xinerama::query_screens(conn) {
                            eprintln!("TRACE: update_geom - before query_screens.reply");
                            if let Ok(screens) = screens_cookie.reply() {
                                eprintln!(
                                    "TRACE: update_geom - got screens, len = {}",
                                    screens.screen_info.len()
                                );
                                let screen_info: Vec<Rect> = screens
                                    .screen_info
                                    .iter()
                                    .map(|s| Rect {
                                        x: s.x_org as i32,
                                        y: s.y_org as i32,
                                        w: s.width as i32,
                                        h: s.height as i32,
                                    })
                                    .collect();

                                eprintln!("TRACE: update_geom - screen_info = {:?}", screen_info);
                                let mut unique: Vec<Rect> = Vec::new();
                                for info in &screen_info {
                                    if is_unique_geom(&unique, info) {
                                        unique.push(*info);
                                    }
                                }

                                let nn = unique.len();
                                eprintln!("TRACE: update_geom - unique.len = {}", nn);
                                let n = {
                                    let g = get_globals();
                                    g.monitors.len()
                                };
                                eprintln!("TRACE: update_geom - current monitors.len = {}", n);

                                {
                                    eprintln!("TRACE: update_geom - before create_monitor loop");
                                    let g = get_globals_mut();
                                    // Get values before the loop to use in create_monitor
                                    let mfact = g.mfact;
                                    let nmaster = g.nmaster;
                                    let showbar = g.showbar;
                                    let topbar = g.topbar;
                                    while g.monitors.len() < nn {
                                        g.monitors.push(create_monitor_with_values(
                                            mfact, nmaster, showbar, topbar,
                                        ));
                                    }
                                    eprintln!("TRACE: update_geom - after create_monitor loop");
                                }

                                eprintln!("TRACE: update_geom - before unique.iter().enumerate");
                                // Track which monitors need bar position updates
                                let mut monitors_need_bar_update: Vec<usize> = Vec::new();

                                for (i, info) in unique.iter().enumerate() {
                                    eprintln!("TRACE: update_geom - iter i = {}", i);
                                    let g = get_globals_mut();
                                    if i >= n {
                                        dirty = true;
                                    }

                                    if let Some(ref mut m) = g.monitors.get_mut(i) {
                                        eprintln!("TRACE: update_geom - got monitor {}", i);
                                        if i >= n
                                            || m.monitor_rect.x != info.x
                                            || m.monitor_rect.y != info.y
                                            || m.monitor_rect.w != info.w
                                            || m.monitor_rect.h != info.h
                                        {
                                            eprintln!(
                                                "TRACE: update_geom - updating monitor {}",
                                                i
                                            );
                                            dirty = true;
                                            m.num = i as i32;
                                            m.monitor_rect = *info;
                                            m.work_rect = *info;
                                            monitors_need_bar_update.push(i);
                                        }
                                    }
                                }

                                for idx in &monitors_need_bar_update {
                                    let g = get_globals_mut();
                                    if let Some(ref mut m) = g.monitors.get_mut(*idx) {
                                        update_bar_pos(m);
                                    }
                                }
                                eprintln!(
                                    "TRACE: update_geom - {} monitors had bar update",
                                    monitors_need_bar_update.len()
                                );

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

                                        let g = get_globals_mut();
                                        if let Some(ref mut c) = g.clients.get_mut(&win) {
                                            c.mon_id = Some(0);
                                        }

                                        attach(win);
                                        attach_stack(win);
                                    }

                                    let g = get_globals_mut();
                                    if g.selmon == i {
                                        g.selmon = 0;
                                    }

                                    cleanup_monitor(i);
                                }

                                if dirty {
                                    let g = get_globals_mut();
                                    g.selmon = 0;

                                    if let Some(m) = win_to_mon(x11.screen_num as u32) {
                                        let g = get_globals_mut();
                                        g.selmon = m;
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
        let (sw, sh) = (g.sw, g.sh);
        let g = get_globals_mut();
        g.monitors.push(create_monitor());
        if let Some(ref mut m) = g.monitors.first_mut() {
            m.num = 0;
            m.monitor_rect.x = 0;
            m.monitor_rect.y = 0;
            m.monitor_rect.w = sw;
            m.monitor_rect.h = sh;
            m.work_rect.x = 0;
            m.work_rect.y = 0;
            m.work_rect.w = sw;
            m.work_rect.h = sh;
            update_bar_pos(m);
        }
        g.selmon = 0;
        dirty = true;
    } else {
        let sw = g.sw;
        let sh = g.sh;
        let needs_update = g
            .monitors
            .first()
            .map(|m| m.monitor_rect.w != sw || m.monitor_rect.h != sh)
            .unwrap_or(false);

        if needs_update {
            dirty = true;
            let g = get_globals_mut();
            if let Some(ref mut m) = g.monitors.first_mut() {
                m.monitor_rect.w = sw;
                m.monitor_rect.h = sh;
                m.work_rect.w = sw;
                m.work_rect.h = sh;
                update_bar_pos(m);
            }
        }
    }

    dirty
}

pub fn arrange(m: Option<MonitorId>) {
    crate::layouts::arrange(m);
}

pub fn arrange_mon(m: &mut Monitor) {
    crate::layouts::arrange_monitor(m);
}

pub fn restack(m: &mut Monitor) {
    crate::layouts::restack(m);
}

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

fn get_selected_client(mon_id: MonitorId) -> Option<Client> {
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

/// Get the current tag for a monitor.
/// Returns None if the monitor's current_tag is invalid.
pub fn get_current_tag<'a>(mon: &Monitor, tags: &'a TagSet) -> Option<&'a Tag> {
    if mon.current_tag > 0 && mon.current_tag <= tags.tags.len() {
        Some(&tags.tags[mon.current_tag - 1])
    } else {
        None
    }
}

/// Get the current tag mutably.
pub fn get_current_tag_mut<'a>(mon: &Monitor, tags: &'a mut TagSet) -> Option<&'a mut Tag> {
    if mon.current_tag > 0 && mon.current_tag <= tags.tags.len() {
        Some(&mut tags.tags[mon.current_tag - 1])
    } else {
        None
    }
}

/// Get the current layout symbol for a monitor.
pub fn get_current_ltsymbol(mon: &Monitor, tags: &TagSet, layouts: &[&dyn Layout]) -> String {
    if let Some(tag) = get_current_tag(mon, tags) {
        if let Some(lt_idx) = tag.ltidxs[tag.sellt as usize] {
            layouts
                .get(lt_idx)
                .map(|l| l.symbol().to_string())
                .unwrap_or_else(|| "[]=".to_string())
        } else {
            "[]=".to_string()
        }
    } else {
        "[]=".to_string()
    }
}

/// Check if the current tag's showbar is enabled.
pub fn get_current_showbar(mon: &Monitor, tags: &TagSet) -> bool {
    get_current_tag(mon, tags)
        .map(|t| t.showbar)
        .unwrap_or(true)
}

/// Check if the current layout is tiling (sellt == 0).
pub fn is_current_layout_tiling(mon: &Monitor, tags: &TagSet) -> bool {
    get_current_tag(mon, tags)
        .map(|t| t.sellt == 0)
        .unwrap_or(true)
}

fn reset_sticky(_c: &mut Client) {}

fn warp_cursor_to_client(_win: Window) {}
