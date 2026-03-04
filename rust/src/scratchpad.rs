use crate::backend::BackendKind;
use crate::client::{attach, attach_stack, detach, detach_stack};
use crate::contexts::WmCtx;
use crate::focus::warp_cursor_to_client;
use crate::layouts::{arrange, restack};
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

const SCRATCHPAD_CLASS_PREFIX: &[u8] = b"scratchpad_";
const SCRATCHPAD_CLASS_PREFIX_LEN: usize = 11;

pub fn unhide_one(ctx: &mut WmCtx) -> bool {
    let clients: Vec<WindowId> = ctx.g.clients.keys().copied().collect();

    for win in clients {
        if ctx.g.clients.is_hidden(win) {
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

    let selected_window = match ctx.g.selected_monitor().sel {
        Some(w) => w,
        None => return,
    };

    if scratchpad_find(ctx, name).is_some() {
        return;
    }

    let (was_scratchpad, old_tags) = {
        if let Some(c) = ctx.g.clients.get(&selected_window) {
            let was_scratchpad = c.is_scratchpad();
            let old_tags = if !was_scratchpad { c.tags } else { 0 };
            (was_scratchpad, old_tags)
        } else {
            return;
        }
    };

    {
        if let Some(client) = ctx.g.clients.get_mut(&selected_window) {
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

    let selected_monitor_id = ctx.g.selected_monitor_id();
    crate::focus::focus_soft(ctx, None);
    if !ctx.g.monitors.is_empty() {
        arrange(ctx, Some(selected_monitor_id));
    }
}

pub fn scratchpad_unmake(ctx: &mut WmCtx) {
    let selected_window = match ctx.g.selected_monitor().sel {
        Some(w) => w,
        None => return,
    };

    let (is_scratchpad, restore_tags, monitor_id, monitor_tags) = {
        let monitor_tags =
            ctx.g.selected_monitor().tagset[ctx.g.selected_monitor().seltags as usize];

        if let Some(c) = ctx.g.clients.get(&selected_window) {
            (
                c.is_scratchpad(),
                c.scratchpad_restore_tags,
                c.monitor_id,
                monitor_tags,
            )
        } else {
            return;
        }
    };

    if !is_scratchpad {
        return;
    }

    {
        if let Some(client) = ctx.g.clients.get_mut(&selected_window) {
            client.scratchpad_name.clear();
            client.issticky = false;
            client.tags = if restore_tags != 0 {
                restore_tags
            } else {
                monitor_tags
            };
            client.scratchpad_restore_tags = 0;
        }
    }

    if let Some(mid) = monitor_id {
        arrange(ctx, Some(mid));
    }
}

pub(crate) fn scratchpad_show_name(ctx: &mut WmCtx, name: &str) {
    let found = match scratchpad_find(ctx, name) {
        Some(w) => w,
        None => return,
    };

    let (current_mon, target_mon) = {
        let current_mon = ctx.g.selected_monitor_id();
        let target_mon = ctx
            .g
            .clients
            .get(&found)
            .and_then(|c| c.monitor_id)
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
        detach(ctx, found);
        detach_stack(ctx, found);

        {
            if let Some(client) = ctx.g.clients.get_mut(&found) {
                client.monitor_id = Some(current_mon);
            }
        }

        attach(ctx, found);
        attach_stack(ctx, found);
    }

    let focusfollowsmouse = ctx.g.focusfollowsmouse;
    if !ctx.g.monitors.is_empty() {
        let mid = ctx.g.selected_monitor_id();
        crate::focus::focus_soft(ctx, Some(found));
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

    let (is_sticky, monitor_id) = {
        if let Some(c) = ctx.g.clients.get(&found) {
            (c.issticky, c.monitor_id)
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

    crate::focus::focus_soft(ctx, None);
    if let Some(mid) = monitor_id {
        arrange(ctx, Some(mid));
    }
}

pub fn scratchpad_toggle(ctx: &mut WmCtx, name: Option<&str>) {
    let name = match name {
        Some(n) => n,
        None => return,
    };

    let is_overview = !ctx.g.selected_monitor().is_tiling_layout();

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
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    let root = ctx.g.cfg.root;

    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return;
    };

    if !name.is_empty() && name != "all" {
        let found = scratchpad_find(ctx, name);
        let visible = found
            .map(|w| ctx.g.clients.get(&w).map(|c| c.issticky).unwrap_or(false))
            .unwrap_or(false);

        let status = format!("ipc:scratchpad:{}:{}", name, if visible { 1 } else { 0 });

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
        return;
    }

    let mut status = String::from("ipc:scratchpads:");
    let mut first = true;

    for (_i, mon) in ctx.g.monitors_iter() {
        for (_c_win, c) in mon.iter_clients(&*ctx.g.clients) {
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
        }
    }

    if first {
        status.push_str("none");
    }

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

fn scratchpad_find(ctx: &WmCtx, name: &str) -> Option<WindowId> {
    if name.is_empty() {
        return None;
    }

    for (_i, mon) in ctx.g.monitors_iter() {
        for (c_win, c) in mon.iter_clients(&*ctx.g.clients) {
            if c.is_scratchpad() && c.scratchpad_name == name {
                return Some(c_win);
            }
        }
    }
    None
}
