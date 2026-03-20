use crate::contexts::WmCtx;
use crate::globals::Globals;
use crate::layouts::{arrange, restack};
use crate::types::{SCRATCHPAD_MASK, WindowId};
use bincode::{Decode, Encode};

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct ScratchpadInfo {
    pub name: String,
    pub visible: bool,
    pub window_id: Option<u32>,
    pub monitor: Option<usize>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub floating: bool,
    pub fullscreen: bool,
}

pub fn unhide_one(ctx: &mut WmCtx) -> bool {
    let clients: Vec<WindowId> = ctx
        .core_mut()
        .globals_mut()
        .clients
        .keys()
        .copied()
        .collect();

    for win in clients {
        if ctx.core_mut().globals_mut().clients.is_hidden(win) {
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

    let Some(selected_window) = ctx.core_mut().globals_mut().selected_monitor().sel else {
        return;
    };

    if scratchpad_find(ctx.core().globals(), name).is_some() {
        return;
    }

    let Some(client) = ctx
        .core_mut()
        .globals_mut()
        .clients
        .get_mut(&selected_window)
    else {
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

    let selected_monitor_id = ctx.core_mut().globals_mut().selected_monitor_id();
    crate::focus::focus_soft(ctx, None);
    if !ctx.core_mut().globals_mut().monitors.is_empty() {
        arrange(ctx, Some(selected_monitor_id));
    }
}

pub fn scratchpad_unmake(ctx: &mut WmCtx) {
    let Some(selected_window) = ctx.core_mut().globals_mut().selected_monitor().sel else {
        return;
    };

    let monitor_tags = ctx
        .core_mut()
        .globals_mut()
        .selected_monitor()
        .selected_tags();

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

pub fn scratchpad_show_name(ctx: &mut WmCtx, name: &str) -> Result<String, String> {
    let Some(found) = scratchpad_find(ctx.core().globals(), name) else {
        return Err(format!("scratchpad '{}' not found", name));
    };

    let was_sticky = ctx
        .core()
        .globals()
        .clients
        .get(&found)
        .is_some_and(|c| c.issticky);

    if was_sticky {
        return Ok(format!("scratchpad '{}' is already visible", name));
    }

    let current_mon = ctx.core_mut().globals_mut().selected_monitor_id();
    let target_mon = ctx
        .core()
        .globals()
        .clients
        .get(&found)
        .map(|c| c.monitor_id)
        .unwrap_or(current_mon);

    if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&found) {
        client.issticky = true;
        client.is_floating = true;
    }

    if target_mon != current_mon {
        ctx.core_mut().globals_mut().detach(found);
        ctx.core_mut().globals_mut().detach_stack(found);

        if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&found) {
            client.monitor_id = current_mon;
        }

        ctx.core_mut().globals_mut().attach(found);
        ctx.core_mut().globals_mut().attach_stack(found);
    }

    let focusfollowsmouse = ctx.core_mut().globals_mut().behavior.focus_follows_mouse;
    if !ctx.core_mut().globals_mut().monitors.is_empty() {
        let mid = ctx.core_mut().globals_mut().selected_monitor_id();
        crate::focus::focus_soft(ctx, Some(found));
        arrange(ctx, Some(mid));
        restack(ctx, mid);
        if focusfollowsmouse {
            ctx.warp_cursor_to_client(found);
        }
    }

    Ok(format!("shown scratchpad '{}'", name))
}

pub fn scratchpad_show_all(ctx: &mut WmCtx) -> Option<String> {
    let scratchpad_names: Vec<String> = ctx
        .core()
        .globals()
        .monitors_iter_all()
        .flat_map(|mon| mon.iter_clients(ctx.core().globals().clients.map()))
        .filter(|(_, c)| c.is_scratchpad() && !c.issticky)
        .map(|(_, c)| c.scratchpad_name.clone())
        .collect();

    let mut shown_count = 0;

    for name in scratchpad_names {
        match scratchpad_show_name(ctx, &name) {
            Ok(_) => {
                shown_count += 1;
            }
            Err(_) => {}
        }
    }

    if shown_count > 0 {
        Some(format!(
            "shown {} scratchpad{}",
            shown_count,
            if shown_count == 1 { "" } else { "s" }
        ))
    } else {
        None
    }
}

pub fn scratchpad_hide_name(ctx: &mut WmCtx, name: &str) {
    let Some(found) = scratchpad_find(ctx.core().globals(), name) else {
        return;
    };

    let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&found) else {
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

    let is_overview = !ctx
        .core_mut()
        .globals_mut()
        .selected_monitor()
        .is_tiling_layout();

    if is_overview {
        return;
    }

    let found = match scratchpad_find(ctx.core().globals(), name) {
        Some(w) => w,
        None => return,
    };

    let Some(client) = ctx.core().globals().clients.get(&found) else {
        return;
    };
    let is_sticky = client.issticky;

    if is_sticky {
        scratchpad_hide_name(ctx, name);
    } else {
        let _ = scratchpad_show_name(ctx, name);
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

fn collect_scratchpad_info(g: &Globals) -> Vec<ScratchpadInfo> {
    let mut scratchpads = Vec::new();

    for mon in g.monitors_iter_all() {
        for (c_win, c) in mon.iter_clients(g.clients.map()) {
            if c.is_scratchpad() {
                scratchpads.push(ScratchpadInfo {
                    name: c.scratchpad_name.clone(),
                    visible: c.issticky,
                    window_id: Some(c_win.0),
                    monitor: Some(c.monitor_id),
                    x: Some(c.geo.x),
                    y: Some(c.geo.y),
                    width: Some(c.geo.w),
                    height: Some(c.geo.h),
                    floating: c.is_floating,
                    fullscreen: c.is_fullscreen,
                });
            }
        }
    }

    scratchpads
}

pub fn scratchpad_list_json(g: &Globals) -> String {
    let scratchpads = collect_scratchpad_info(g);
    serde_json::to_string_pretty(&scratchpads).unwrap_or_else(|_| "[]".to_string())
}

/// List all scratchpads with detailed information.
///
/// Returns a formatted string like:
/// ```text
/// * term     visible    window: 12345    monitor: 0    800x600+100+50    floating
///   music    hidden     window: 67890    monitor: 1    400x300+200+100
/// ```
pub fn scratchpad_list(g: &Globals) -> String {
    let scratchpads = collect_scratchpad_info(g);

    if scratchpads.is_empty() {
        return "no scratchpads".to_string();
    }

    let mut out = String::new();

    for sp in scratchpads {
        if !out.is_empty() {
            out.push('\n');
        }

        let marker = if sp.visible { "* " } else { "  " };
        let status = if sp.visible { "visible" } else { "hidden" };

        let geometry =
            if let (Some(w), Some(h), Some(x), Some(y)) = (sp.width, sp.height, sp.x, sp.y) {
                format!("{}x{}+{}+{}", w, h, x, y)
            } else {
                "unknown geometry".to_string()
            };

        let window_str = if let Some(wid) = sp.window_id {
            format!("window: {}", wid)
        } else {
            "no window".to_string()
        };

        let monitor_str = if let Some(mon) = sp.monitor {
            format!("monitor: {}", mon)
        } else {
            "no monitor".to_string()
        };

        let flags = if sp.fullscreen && sp.floating {
            " fullscreen, floating"
        } else if sp.fullscreen {
            " fullscreen"
        } else if sp.floating {
            " floating"
        } else {
            ""
        };

        out.push_str(&format!(
            "{}{:<12} {:<8}  {:<18} {:<14} {}{}",
            marker, sp.name, status, window_str, monitor_str, geometry, flags
        ));
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
