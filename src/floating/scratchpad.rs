use crate::contexts::WmCtx;
use crate::globals::Globals;
use crate::layouts::{arrange, restack};
use crate::types::*;

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

    if scratchpad_find(ctx.g(), name).is_some() {
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

    if !client.is_floating {
        client.is_floating = true;
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

    let Some(client) = ctx.client(selected_window) else {
        return;
    };
    if !client.is_scratchpad() {
        return;
    }
    let restore_tags = client.scratchpad_restore_tags;
    let monitor_id = client.monitor_id;

    if let Some(client) = ctx.client_mut(selected_window) {
        client.scratchpad_name.clear();
        client.issticky = false;
        client.tags = if restore_tags != 0 {
            restore_tags
        } else {
            monitor_tags
        };
        client.scratchpad_restore_tags = 0;
    }

    arrange(ctx, Some(monitor_id));
}

pub fn scratchpad_show_name(ctx: &mut WmCtx, name: &str) {
    let Some(found) = scratchpad_find(ctx.g(), name) else {
        return;
    };

    let current_mon = ctx.g_mut().selected_monitor_id();
    let target_mon = ctx
        .g
        .clients
        .get(&found)
        .map(|c| c.monitor_id)
        .unwrap_or(current_mon);

    if let Some(client) = ctx.g_mut().clients.get_mut(&found) {
        client.issticky = true;
        client.is_floating = true;
    }

    if target_mon != current_mon {
        ctx.g_mut().detach(found);
        ctx.g_mut().detach_stack(found);

        if let Some(client) = ctx.g_mut().clients.get_mut(&found) {
            client.monitor_id = current_mon;
        }

        ctx.g_mut().attach(found);
        ctx.g_mut().attach_stack(found);
    }

    let focusfollowsmouse = ctx.g_mut().behavior.focus_follows_mouse;
    if !ctx.g_mut().monitors.is_empty() {
        let mid = ctx.g_mut().selected_monitor_id();
        crate::focus::focus_soft(ctx, Some(found));
        arrange(ctx, Some(mid));
        restack(ctx, mid);
        if focusfollowsmouse {
            ctx.warp_cursor_to_client(found);
        }
    }
}

pub fn scratchpad_hide_name(ctx: &mut WmCtx, name: &str) {
    let Some(found) = scratchpad_find(ctx.g(), name) else {
        return;
    };

    let Some(client) = ctx.g_mut().clients.get_mut(&found) else {
        return;
    };
    if !client.issticky {
        return;
    }
    let monitor_id = client.monitor_id;

    client.issticky = false;
    client.tags = SCRATCHPAD_MASK;

    crate::focus::focus_soft(ctx, None);
    arrange(ctx, Some(monitor_id));
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

    let found = match scratchpad_find(ctx.g(), name) {
        Some(w) => w,
        None => return,
    };

    let Some(client) = ctx.g().clients.get(&found) else {
        return;
    };
    let is_sticky = client.issticky;

    if is_sticky {
        scratchpad_hide_name(ctx, name);
    } else {
        scratchpad_show_name(ctx, name);
    }
}

pub fn scratchpad_status(g: &Globals, name: &str) -> String {
    if !name.is_empty() && name != "all" {
        let found = scratchpad_find(g, name);
        let visible = found
            .and_then(|w| g.clients.get(&w))
            .is_some_and(|c| c.issticky);

        return format!("ipc:scratchpad:{}:{}", name, if visible { 1 } else { 0 });
    }

    let mut status = String::from("ipc:scratchpads:");
    let mut first = true;

    for mon in g.monitors_iter_all() {
        for (_c_win, c) in mon.iter_clients(g.clients.map()) {
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

    status
}

/// List all scratchpads with their visibility status.
///
/// Returns a formatted string like:
/// ```text
/// * term     (visible)
///   music    (hidden)
/// ```
pub fn scratchpad_list(g: &Globals) -> String {
    let mut out = String::new();
    let mut first = true;

    for mon in g.monitors_iter_all() {
        for (_c_win, c) in mon.iter_clients(g.clients.map()) {
            if c.is_scratchpad() {
                if !first {
                    out.push('\n');
                }
                let marker = if c.issticky { "* " } else { "  " };
                let status = if c.issticky { "(visible)" } else { "(hidden)" };
                out.push_str(&format!("{}{} {}", marker, c.scratchpad_name, status));
                first = false;
            }
        }
    }

    if first {
        out.push_str("no scratchpads");
    }

    out
}

fn scratchpad_find(g: &Globals, name: &str) -> Option<WindowId> {
    if name.is_empty() {
        return None;
    }

    for mon in g.monitors_iter_all() {
        for (c_win, c) in mon.iter_clients(g.clients.map()) {
            if c.is_scratchpad() && c.scratchpad_name == name {
                return Some(c_win);
            }
        }
    }
    None
}
