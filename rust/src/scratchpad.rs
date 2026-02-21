use crate::client::{attach, attach_stack, detach, detach_stack, is_visible};
use crate::focus::{focus, warp_cursor_to_client};
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::layouts::arrange;
use crate::monitor::restack;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

const SCRATCHPAD_CLASS_PREFIX: &[u8] = b"scratchpad_";
const SCRATCHPAD_CLASS_PREFIX_LEN: usize = 11;

pub fn hide_window(win: Window) {
    crate::client::hide(win);
}

pub fn unhide_one() -> bool {
    let clients: Vec<Window> = {
        let globals = get_globals();
        globals.clients.keys().copied().collect()
    };

    for win in clients {
        if crate::client::is_hidden(win) {
            crate::client::show(win);
            return true;
        }
    }
    false
}

fn extract_name(arg: &Arg) -> Option<String> {
    let ptr = arg.v? as *const u8;
    let slice = unsafe {
        let len = (0..SCRATCHPAD_NAME_LEN)
            .find(|&i| *ptr.add(i) == 0)
            .unwrap_or(SCRATCHPAD_NAME_LEN);
        std::slice::from_raw_parts(ptr, len)
    };
    if slice.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(slice).into_owned())
    }
}

fn scratchpad_find(name: &str) -> Option<Window> {
    if name.is_empty() {
        return None;
    }

    let globals = get_globals();
    for mon in &globals.monitors {
        let mut current = mon.clients;
        while let Some(c_win) = current {
            if let Some(c) = globals.clients.get(&c_win) {
                if c.is_scratchpad() && c.scratchpad_name == name {
                    return Some(c_win);
                }
                current = c.next;
            } else {
                break;
            }
        }
    }
    None
}

pub fn scratchpad_any_visible(mon: &MonitorInner) -> bool {
    let globals = get_globals();
    let mut current = mon.clients;
    while let Some(c_win) = current {
        if let Some(c) = globals.clients.get(&c_win) {
            if c.is_scratchpad() && c.issticky {
                return true;
            }
            current = c.next;
        } else {
            break;
        }
    }
    false
}

pub fn scratchpad_identify_client(c: &mut Client) {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    let class_hint = conn.get_property(false, c.win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024);

    let Ok(cookie) = class_hint else { return };
    let Ok(reply) = cookie.reply() else { return };

    let data: Vec<u8> = reply.value8().map(|v| v.collect()).unwrap_or_default();
    let parts: Vec<&[u8]> = data.split(|&b| b == 0).filter(|s| !s.is_empty()).collect();

    let match_name: Option<&[u8]> = parts.iter().find_map(|part| {
        if part.len() > SCRATCHPAD_CLASS_PREFIX_LEN
            && &part[..SCRATCHPAD_CLASS_PREFIX_LEN] == SCRATCHPAD_CLASS_PREFIX
        {
            Some(&part[SCRATCHPAD_CLASS_PREFIX_LEN..])
        } else {
            None
        }
    });

    if let Some(name) = match_name {
        c.scratchpad_name = String::from_utf8_lossy(name).into_owned();
        c.scratchpad_restore_tags = 0;
        c.tags = SCRATCHPAD_MASK;
        c.issticky = true;
        c.isfloating = true;
    }
}

pub fn scratchpad_make(arg: &Arg) {
    let name = match extract_name(arg) {
        Some(n) => n,
        None => return,
    };

    let sel_win = {
        let globals = get_globals();
        let selmon_id = match globals.selmon {
            Some(id) => id,
            None => return,
        };
        globals.monitors.get(selmon_id).and_then(|m| m.sel)
    };

    let sel_win = match sel_win {
        Some(w) => w,
        None => return,
    };

    if scratchpad_find(&name).is_some() {
        return;
    }

    let (was_scratchpad, old_tags) = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&sel_win) {
            let was_sp = c.is_scratchpad();
            let old_tags = if !was_sp { c.tags } else { 0 };
            (was_sp, old_tags)
        } else {
            return;
        }
    };

    {
        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&sel_win) {
            client.scratchpad_name = name;

            if !was_scratchpad {
                client.scratchpad_restore_tags = old_tags;
            }

            client.tags = SCRATCHPAD_MASK;
            client.issticky = false;

            if !client.isfloating {
                client.isfloating = true;
            }
        }
    }

    focus(None);

    let mon_id = {
        let globals = get_globals();
        globals.selmon
    };
    if let Some(mid) = mon_id {
        arrange(Some(mid));
    }
}

pub fn scratchpad_unmake(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        let selmon_id = match globals.selmon {
            Some(id) => id,
            None => return,
        };
        globals.monitors.get(selmon_id).and_then(|m| m.sel)
    };

    let sel_win = match sel_win {
        Some(w) => w,
        None => return,
    };

    let (is_scratchpad, restore_tags, mon_id, mon_tags) = {
        let globals = get_globals();
        let selmon_id = globals.selmon.unwrap_or(0);
        let mon_tags = globals
            .monitors
            .get(selmon_id)
            .map(|m| m.tagset[m.seltags as usize])
            .unwrap_or(1);

        if let Some(c) = globals.clients.get(&sel_win) {
            (
                c.is_scratchpad(),
                c.scratchpad_restore_tags,
                c.mon_id,
                mon_tags,
            )
        } else {
            return;
        }
    };

    if !is_scratchpad {
        return;
    }

    {
        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&sel_win) {
            client.scratchpad_name.clear();
            client.issticky = false;
            client.tags = if restore_tags != 0 {
                restore_tags
            } else {
                mon_tags
            };
            client.scratchpad_restore_tags = 0;
        }
    }

    if let Some(mid) = mon_id {
        arrange(Some(mid));
    }
}

pub fn scratchpad_show(arg: &Arg) {
    if let Some(name) = extract_name(arg) {
        scratchpad_show_name(&name);
    }
}

pub(crate) fn scratchpad_show_name(name: &str) {
    let found = match scratchpad_find(name) {
        Some(w) => w,
        None => return,
    };

    let (current_mon, target_mon) = {
        let globals = get_globals();
        let current_mon = globals.selmon;
        let target_mon = globals.clients.get(&found).and_then(|c| c.mon_id);
        (current_mon, target_mon)
    };

    {
        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&found) {
            client.issticky = true;
            client.isfloating = true;
        }
    }

    if target_mon != current_mon {
        detach(found);
        detach_stack(found);

        {
            let mut globals = get_globals_mut();
            if let Some(client) = globals.clients.get_mut(&found) {
                client.mon_id = current_mon;
            }
        }

        attach(found);
        attach_stack(found);
    }

    focus(Some(found));

    let selmon_id = {
        let globals = get_globals();
        globals.selmon
    };
    if let Some(mid) = selmon_id {
        arrange(Some(mid));
        let mut globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(mid) {
            restack(mon);
        }
    }

    let focusfollowsmouse = {
        let globals = get_globals();
        globals.focusfollowsmouse
    };

    if focusfollowsmouse {
        warp_cursor_to_client(found);
    }
}

pub fn scratchpad_hide(arg: &Arg) {
    if let Some(name) = extract_name(arg) {
        scratchpad_hide_name(&name);
    }
}

pub(crate) fn scratchpad_hide_name(name: &str) {
    let found = match scratchpad_find(name) {
        Some(w) => w,
        None => return,
    };

    let (is_sticky, mon_id) = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&found) {
            (c.issticky, c.mon_id)
        } else {
            return;
        }
    };

    if !is_sticky {
        return;
    }

    {
        let mut globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&found) {
            client.issticky = false;
            client.tags = SCRATCHPAD_MASK;
        }
    }

    focus(None);

    if let Some(mid) = mon_id {
        arrange(Some(mid));
    }
}

pub fn scratchpad_toggle(arg: &Arg) {
    let name = match extract_name(arg) {
        Some(n) => n,
        None => return,
    };

    let is_overview = {
        let globals = get_globals();
        let selmon_id = globals.selmon.unwrap_or(0);
        if let Some(mon) = globals.monitors.get(selmon_id) {
            mon.sellt != 0
        } else {
            false
        }
    };

    if is_overview {
        return;
    }

    let found = match scratchpad_find(&name) {
        Some(w) => w,
        None => return,
    };

    let is_sticky = {
        let globals = get_globals();
        globals
            .clients
            .get(&found)
            .map(|c| c.issticky)
            .unwrap_or(false)
    };

    if is_sticky {
        scratchpad_hide_name(&name);
    } else {
        scratchpad_show_name(&name);
    }
}

pub fn scratchpad_status(arg: &Arg) {
    let name = extract_name(arg).unwrap_or_default();

    let globals = get_globals();
    let root = globals.root;

    if !name.is_empty() && name != "all" {
        let found = scratchpad_find(&name);
        let visible = found
            .map(|w| globals.clients.get(&w).map(|c| c.issticky).unwrap_or(false))
            .unwrap_or(false);

        let status = format!("ipc:scratchpad:{}:{}", name, if visible { 1 } else { 0 });

        drop(globals);

        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = conn.change_property(
                x11rb::protocol::xproto::PropMode::REPLACE,
                root,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                8u8,
                status.len() as u32,
                status.as_bytes(),
            );
            let _ = conn.flush();
        }
        return;
    }

    let mut status = String::from("ipc:scratchpads:");
    let mut first = true;

    for mon in &globals.monitors {
        let mut current = mon.clients;
        while let Some(c_win) = current {
            if let Some(c) = globals.clients.get(&c_win) {
                if c.is_scratchpad() {
                    if !first {
                        status.push(',');
                    }
                    status.push_str(&format!(
                        "{}={}",
                        c.scratchpad_name,
                        if c.issticky { 1 } else { 0 }
                    ));
                    first = false;
                }
                current = c.next;
            } else {
                break;
            }
        }
    }

    if first {
        status.push_str("none");
    }

    drop(globals);

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.change_property(
            x11rb::protocol::xproto::PropMode::REPLACE,
            root,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            8u8,
            status.len() as u32,
            status.as_bytes(),
        );
        let _ = conn.flush();
    }
}
