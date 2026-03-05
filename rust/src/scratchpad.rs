use crate::backend::BackendKind;
use crate::client::{attach, attach_stack, detach, detach_stack};
use crate::contexts::WmCtx;
use crate::focus::warp_cursor_to_client_x11;
use crate::layouts::{arrange, restack};
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

const SCRATCHPAD_CLASS_PREFIX: &[u8] = b"scratchpad_";
const SCRATCHPAD_CLASS_PREFIX_LEN: usize = 11;

pub fn unhide_one(ctx: &mut WmCtx) -> bool {
    let clients: Vec<WindowId> = ctx.g_mut().clients.keys().copied().collect();

    for win in clients {
        if ctx.g_mut().clients.is_hidden(win) {
            crate::client::show(ctx, win);
            return true;
        }
    }
    false
}

pub fn scratchpad_make(ctx: &mut WmCtx, name: Option<&str>) {
    let Some(name) = name else { return };
    if name.is_empty() {
        return;
    }

    let Some(selected_window) = ctx.g_mut().selected_monitor().sel else {
        return;
    };

    if scratchpad_find(ctx, name).is_some() {
        return;
    }

    let Some(client) = ctx.g_mut().clients.get_mut(&selected_window) else {
        return;
    };

    let was_scratchpad = client.is_scratchpad();
    let old_tags = if was_scratchpad { 0 } else { client.tags };

    client.scratchpad_name = name.to_string();

    if !was_scratchpad {
        client.scratchpad_restore_tags = old_tags;
    }

    client.tags = SCRATCHPAD_MASK;
    client.issticky = false;

    if !client.isfloating {
        client.isfloating = true;
    }

    let selected_monitor_id = ctx.g_mut().selected_monitor_id();
    crate::focus::focus_soft(ctx, None);
    if !ctx.g_mut().monitors.is_empty() {
        arrange(ctx, Some(selected_monitor_id));
    }
}

pub fn scratchpad_unmake(ctx: &mut WmCtx) {
    let Some(selected_window) = ctx.g_mut().selected_monitor().sel else {
        return;
    };

    let monitor_tags = ctx.g_mut().selected_monitor().selected_tags();

    let Some(client) = ctx.g_mut().clients.get(&selected_window) else {
        return;
    };
    if !client.is_scratchpad() {
        return;
    }
    let restore_tags = client.scratchpad_restore_tags;
    let monitor_id = client.monitor_id;

    if let Some(client) = ctx.g_mut().clients.get_mut(&selected_window) {
        client.scratchpad_name.clear();
        client.issticky = false;
        client.tags = if restore_tags != 0 {
            restore_tags
        } else {
            monitor_tags
        };
        client.scratchpad_restore_tags = 0;
    }

    if let Some(mid) = monitor_id {
        arrange(ctx, Some(mid));
    }
}

pub(crate) fn scratchpad_show_name(ctx: &mut WmCtx, name: &str) {
    let Some(found) = scratchpad_find(ctx, name) else {
        return;
    };

    let current_mon = ctx.g_mut().selected_monitor_id();
    let target_mon = ctx
        .g
        .clients
        .get(&found)
        .and_then(|c| c.monitor_id)
        .unwrap_or(current_mon);

    if let Some(client) = ctx.g_mut().clients.get_mut(&found) {
        client.issticky = true;
        client.isfloating = true;
    }

    if target_mon != current_mon {
        detach(ctx, found);
        detach_stack(ctx, found);

        if let Some(client) = ctx.g_mut().clients.get_mut(&found) {
            client.monitor_id = Some(current_mon);
        }

        attach(ctx, found);
        attach_stack(ctx, found);
    }

    let focusfollowsmouse = ctx.g_mut().focusfollowsmouse;
    if !ctx.g_mut().monitors.is_empty() {
        let mid = ctx.g_mut().selected_monitor_id();
        crate::focus::focus_soft(ctx, Some(found));
        arrange(ctx, Some(mid));
        restack(ctx, mid);
        if focusfollowsmouse {
            if let WmCtx::X11(x11) = ctx {
                warp_cursor_to_client_x11(&x11.core, &x11.x11, found);
            }
        }
    }
}

pub(crate) fn scratchpad_hide_name(ctx: &mut WmCtx, name: &str) {
    let Some(found) = scratchpad_find(ctx, name) else {
        return;
    };

    let Some(client) = ctx.g_mut().clients.get(&found) else {
        return;
    };
    if !client.issticky {
        return;
    }
    let monitor_id = client.monitor_id;

    if let Some(client) = ctx.g_mut().clients.get_mut(&found) {
        client.issticky = false;
        client.tags = SCRATCHPAD_MASK;
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

    let is_overview = !ctx.g_mut().selected_monitor().is_tiling_layout();

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
    if ctx.backend_kind_REMOVED() == BackendKind::Wayland {
        return;
    }
    let root = ctx.g().x11.root;

    let conn = match ctx {
        WmCtx::X11(x11) => x11.x11.conn,
        WmCtx::Wayland(_) => return,
    };

    if !name.is_empty() && name != "all" {
        let found = scratchpad_find(ctx, name);
        let visible = found
            .map(|w| ctx.g().clients.get(&w).map(|c| c.issticky).unwrap_or(false))
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

    for mon in ctx.g().monitors_iter_all() {
        for (_c_win, c) in mon.iter_clients(&ctx.g().clients) {
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

    for mon in ctx.g().monitors_iter_all() {
        for (c_win, c) in mon.iter_clients(&ctx.g().clients) {
            if c.is_scratchpad() && c.scratchpad_name == name {
                return Some(c_win);
            }
        }
    }
    None
}
