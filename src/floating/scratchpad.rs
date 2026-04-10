use crate::contexts::WmCtx;
use crate::globals::Globals;
use crate::ipc_types::ScratchpadInitialStatus;
use crate::layouts::arrange;
use crate::types::{MonitorId, WindowId};
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

fn selected_or_explicit_window(ctx: &WmCtx<'_>, window_id: Option<WindowId>) -> Option<WindowId> {
    window_id.or_else(|| ctx.selected_client())
}

fn move_client_to_monitor(g: &mut Globals, win: WindowId, monitor_id: MonitorId) {
    g.detach(win);
    g.detach_stack(win);

    if let Some(client) = g.clients.get_mut(&win) {
        client.monitor_id = monitor_id;
    }

    g.attach(win);
    g.attach_stack(win);
}

fn scratchpad_names(g: &Globals, visible: bool) -> Vec<String> {
    g.clients
        .values()
        .filter(|c| c.is_scratchpad() && c.issticky == visible)
        .map(|c| c.scratchpad_name.clone())
        .collect()
}

pub fn unhide_one(ctx: &mut WmCtx) -> bool {
    let clients: Vec<WindowId> = ctx.core().globals().clients.keys().copied().collect();

    for win in clients {
        let should_unhide = ctx
            .core()
            .globals()
            .clients
            .get(&win)
            .is_some_and(|c| c.is_hidden && !c.is_scratchpad());
        if should_unhide {
            crate::client::show(ctx, win);
            return true;
        }
    }
    false
}

pub fn scratchpad_make(
    ctx: &mut WmCtx,
    name: &str,
    window_id: Option<WindowId>,
    status: ScratchpadInitialStatus,
) {
    if name.is_empty() {
        return;
    }

    let target = selected_or_explicit_window(ctx, window_id);
    let Some(selected_window) = target else {
        return;
    };

    if scratchpad_find(ctx.core().globals(), name).is_some() {
        return;
    }

    let Some(client) = ctx.client_mut(selected_window) else {
        return;
    };

    let was_scratchpad = client.is_scratchpad();
    let old_tags = if was_scratchpad {
        crate::types::TagMask::EMPTY
    } else {
        client.tags
    };

    client.scratchpad_name = name.to_string();

    if !was_scratchpad {
        client.scratchpad_restore_tags = old_tags;
    }

    client.set_tag_mask(crate::types::TagMask::SCRATCHPAD);
    client.issticky = false;

    if !client.is_floating {
        client.is_floating = true;
    }

    crate::client::hide(ctx, selected_window);

    if matches!(status, ScratchpadInitialStatus::Shown) {
        let _ = scratchpad_show_name(ctx, name);
    }
}

pub fn scratchpad_unmake(ctx: &mut WmCtx, window_id: Option<WindowId>) {
    let target = selected_or_explicit_window(ctx, window_id);
    let Some(selected_window) = target else {
        return;
    };

    let monitor_tags = ctx.core().globals().selected_monitor().selected_tags();

    let Some(client) = ctx.client(selected_window) else {
        return;
    };
    if !client.is_scratchpad() {
        return;
    }
    let restore_tags = client.scratchpad_restore_tags;
    let monitor_id = client.monitor_id;

    let mut was_hidden = false;
    if let Some(client) = ctx.client_mut(selected_window) {
        was_hidden = client.is_hidden;
        client.set_tag_mask(if !restore_tags.is_empty() {
            restore_tags
        } else {
            monitor_tags
        });
    }

    if was_hidden {
        crate::client::show(ctx, selected_window);
    } else {
        arrange(ctx, Some(monitor_id));
    }
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

    let current_mon = ctx.core().globals().selected_monitor_id();
    let target_mon = ctx
        .core()
        .globals()
        .clients
        .get(&found)
        .map(|c| c.monitor_id)
        .unwrap_or(current_mon);

    if let Some(client) = ctx.client_mut(found) {
        client.issticky = true;
        client.is_floating = true;
    }

    if target_mon != current_mon {
        move_client_to_monitor(ctx.core_mut().globals_mut(), found, current_mon);
    }

    let focusfollowsmouse = ctx.core().globals().behavior.focus_follows_mouse;

    let is_hidden = ctx
        .core()
        .globals()
        .clients
        .get(&found)
        .map(|c| c.is_hidden)
        .unwrap_or(false);
    if is_hidden {
        crate::client::show(ctx, found);
    } else {
        let mid = ctx.core().globals().selected_monitor_id();
        crate::focus::focus_soft(ctx, Some(found));
        arrange(ctx, Some(mid));
        crate::layouts::restack(ctx, mid);
    }

    if focusfollowsmouse {
        ctx.warp_cursor_to_client(found);
    }

    Ok(format!("shown scratchpad '{}'", name))
}

pub fn scratchpad_show_all(ctx: &mut WmCtx) -> Option<String> {
    let scratchpad_names = scratchpad_names(ctx.core().globals(), false);

    let mut shown_count = 0;

    for name in scratchpad_names {
        if scratchpad_show_name(ctx, &name).is_ok() {
            shown_count += 1;
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

pub fn scratchpad_hide_all(ctx: &mut WmCtx) -> Option<String> {
    let scratchpad_names = scratchpad_names(ctx.core().globals(), true);

    let mut hidden_count = 0;

    for name in scratchpad_names {
        let was_visible = ctx
            .core()
            .globals()
            .clients
            .values()
            .any(|c| c.is_scratchpad() && c.scratchpad_name == name && c.issticky);
        scratchpad_hide_name(ctx, &name);
        if was_visible {
            hidden_count += 1;
        }
    }

    if hidden_count > 0 {
        Some(format!(
            "hid {} scratchpad{}",
            hidden_count,
            if hidden_count == 1 { "" } else { "s" }
        ))
    } else {
        None
    }
}

pub fn scratchpad_hide_name(ctx: &mut WmCtx, name: &str) {
    let Some(found) = scratchpad_find(ctx.core().globals(), name) else {
        return;
    };

    let Some(client) = ctx.client_mut(found) else {
        return;
    };
    if !client.issticky {
        return;
    }

    client.issticky = false;
    client.set_tag_mask(crate::types::TagMask::SCRATCHPAD);

    crate::client::hide(ctx, found);
}

pub fn scratchpad_toggle(ctx: &mut WmCtx, name: Option<&str>) {
    let name = match name {
        Some(n) => n,
        None => return,
    };

    let is_overview = !ctx.core().globals().selected_monitor().is_tiling_layout();

    if is_overview {
        return;
    }

    let found = match scratchpad_find(ctx.core().globals(), name) {
        Some(w) => w,
        None => return,
    };

    let Some(client) = ctx.client(found) else {
        return;
    };
    let is_sticky = client.issticky;

    if is_sticky {
        scratchpad_hide_name(ctx, name);
    } else {
        let _ = scratchpad_show_name(ctx, name);
    }
}

pub fn collect_scratchpad_info(g: &Globals) -> Vec<ScratchpadInfo> {
    let mut scratchpads = Vec::new();

    for c in g.clients.values() {
        if c.is_scratchpad() {
            scratchpads.push(ScratchpadInfo {
                name: c.scratchpad_name.clone(),
                visible: c.issticky,
                window_id: Some(c.win.0),
                monitor: Some(c.monitor_id.index()),
                x: Some(c.geo.x),
                y: Some(c.geo.y),
                width: Some(c.geo.w),
                height: Some(c.geo.h),
                floating: c.is_floating,
                fullscreen: c.is_fullscreen,
            });
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

pub fn scratchpad_find(g: &Globals, name: &str) -> Option<WindowId> {
    if name.is_empty() {
        return None;
    }

    for c in g.clients.values() {
        if c.is_scratchpad() && c.scratchpad_name == name {
            return Some(c.win);
        }
    }
    None
}
