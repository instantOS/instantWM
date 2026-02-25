use crate::client::{attach, attach_stack, detach, detach_stack};
use crate::contexts::WmCtx;
use crate::focus::{focus, warp_cursor_to_client};
use crate::layouts::{arrange, restack};
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

const SCRATCHPAD_CLASS_PREFIX: &[u8] = b"scratchpad_";
const SCRATCHPAD_CLASS_PREFIX_LEN: usize = 11;

pub fn unhide_one(ctx: &mut WmCtx) -> bool {
    let clients: Vec<Window> = ctx.g.clients.keys().copied().collect();

    for win in clients {
        if crate::client::is_hidden(win) {
            crate::client::show(ctx, win);
            return true;
        }
    }
    false
}

pub fn scratchpad_make(ctx: &mut WmCtx, name: Option<&str>) {
    let name = match name {
        Some(n) => n,
        None => return,
    };

    if name.is_empty() {
        return;
    }

    let sel_win = ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.sel);

    let sel_win = match sel_win {
        Some(w) => w,
        None => return,
    };

    if scratchpad_find(ctx, name).is_some() {
        return;
    }

    let (was_scratchpad, old_tags) = {
        if let Some(c) = ctx.g.clients.get(&sel_win) {
            let was_sp = c.is_scratchpad();
            let old_tags = if !was_sp { c.tags } else { 0 };
            (was_sp, old_tags)
        } else {
            return;
        }
    };

    {
        if let Some(client) = ctx.g.clients.get_mut(&sel_win) {
            client.scratchpad_name = name.to_string();

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

    let selmon = ctx.g.selmon;
    focus(ctx, None);
    if !ctx.g.monitors.is_empty() {
        arrange(ctx, Some(selmon));
    }
}

pub fn scratchpad_unmake(ctx: &mut WmCtx) {
    let sel_win = ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.sel);

    let sel_win = match sel_win {
        Some(w) => w,
        None => return,
    };

    let (is_scratchpad, restore_tags, mon_id, mon_tags) = {
        let selmon_id = ctx.g.selmon;
        let mon_tags = ctx
            .g
            .monitors
            .get(selmon_id)
            .map(|m| m.tagset[m.seltags as usize])
            .unwrap_or(1);

        if let Some(c) = ctx.g.clients.get(&sel_win) {
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
        if let Some(client) = ctx.g.clients.get_mut(&sel_win) {
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
        arrange(ctx, Some(mid));
    }
}

pub(crate) fn scratchpad_show_name(ctx: &mut WmCtx, name: &str) {
    let found = match scratchpad_find(ctx, name) {
        Some(w) => w,
        None => return,
    };

    let (current_mon, target_mon) = {
        let current_mon = ctx.g.selmon;
        let target_mon = ctx
            .g
            .clients
            .get(&found)
            .and_then(|c| c.mon_id)
            .unwrap_or(current_mon);
        (current_mon, target_mon)
    };

    {
        if let Some(client) = ctx.g.clients.get_mut(&found) {
            client.issticky = true;
            client.isfloating = true;
        }
    }

    if target_mon != current_mon {
        detach(found);
        detach_stack(found);

        {
            if let Some(client) = ctx.g.clients.get_mut(&found) {
                client.mon_id = Some(current_mon);
            }
        }

        attach(found);
        attach_stack(found);
    }

    let focusfollowsmouse = ctx.g.focusfollowsmouse;
    if !ctx.g.monitors.is_empty() {
        let mid = ctx.g.selmon;
        focus(ctx, Some(found));
        arrange(ctx, Some(mid));
        restack(ctx, mid);
        if focusfollowsmouse {
            warp_cursor_to_client(ctx, found);
        }
    }
}

pub(crate) fn scratchpad_hide_name(ctx: &mut WmCtx, name: &str) {
    let found = match scratchpad_find(ctx, name) {
        Some(w) => w,
        None => return,
    };

    let (is_sticky, mon_id) = {
        if let Some(c) = ctx.g.clients.get(&found) {
            (c.issticky, c.mon_id)
        } else {
            return;
        }
    };

    if !is_sticky {
        return;
    }

    {
        if let Some(client) = ctx.g.clients.get_mut(&found) {
            client.issticky = false;
            client.tags = SCRATCHPAD_MASK;
        }
    }

    focus(ctx, None);
    if let Some(mid) = mon_id {
        arrange(ctx, Some(mid));
    }
}

pub fn scratchpad_toggle(ctx: &mut WmCtx, name: Option<&str>) {
    let name = match name {
        Some(n) => n,
        None => return,
    };

    let is_overview = {
        let selmon_id = ctx.g.selmon;
        if let Some(mon) = ctx.g.monitors.get(selmon_id) {
            !crate::monitor::is_current_layout_tiling(mon, &ctx.g.tags)
        } else {
            false
        }
    };

    if is_overview {
        return;
    }

    let found = match scratchpad_find(ctx, name) {
        Some(w) => w,
        None => return,
    };

    let is_sticky = ctx
        .g
        .clients
        .get(&found)
        .map(|c| c.issticky)
        .unwrap_or(false);

    if is_sticky {
        scratchpad_hide_name(ctx, name);
    } else {
        scratchpad_show_name(ctx, name);
    }
}

pub fn scratchpad_status(ctx: &WmCtx, name: &str) {
    let root = ctx.g.cfg.root;

    if !name.is_empty() && name != "all" {
        let found = scratchpad_find(ctx, name);
        let visible = found
            .map(|w| ctx.g.clients.get(&w).map(|c| c.issticky).unwrap_or(false))
            .unwrap_or(false);

        let status = format!("ipc:scratchpad:{}:{}", name, if visible { 1 } else { 0 });

        if let Some(ref conn) = ctx.x11.conn {
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

    for mon in &ctx.g.monitors {
        let mut current = mon.clients;
        while let Some(c_win) = current {
            if let Some(c) = ctx.g.clients.get(&c_win) {
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

    if let Some(ref conn) = ctx.x11.conn {
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

fn scratchpad_find(ctx: &WmCtx, name: &str) -> Option<Window> {
    if name.is_empty() {
        return None;
    }

    for mon in &ctx.g.monitors {
        let mut current = mon.clients;
        while let Some(c_win) = current {
            if let Some(c) = ctx.g.clients.get(&c_win) {
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

pub fn scratchpad_any_visible(ctx: &WmCtx, mon: &Monitor) -> bool {
    let mut current = mon.clients;
    while let Some(c_win) = current {
        if let Some(c) = ctx.g.clients.get(&c_win) {
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

pub fn scratchpad_identify_client(ctx: &WmCtx, c: &mut Client) {
    let Some(ref conn) = ctx.x11.conn else { return };

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
