use crate::backend::BackendOps;
use crate::client::save_border_width;
use crate::constants::animation::EMPHASIZED_FRAME_COUNT;
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::globals::Globals;
use crate::ipc_types::ScratchpadInitialStatus;
use crate::layouts::arrange;
use crate::types::input::EdgeDirection;
use crate::types::{MonitorId, Rect, WindowId};
use bincode::{Decode, Encode};

const EDGE_MARGIN_X: i32 = 20;
const EDGE_MARGIN_Y: i32 = 40;
const EDGE_INSET_X: i32 = 40;
const EDGE_INSET_Y: i32 = 80;

pub const OVERLAY_NAME: &str = "instantwm_overlay_scratch";

/// Positioning info for the edge slide-in animation.
#[derive(Debug, Clone, Copy)]
struct EdgePositionInfo {
    direction: EdgeDirection,
    /// Monitor rectangle (position and total size).
    monitor_rect: Rect,
    /// Work area width (excluding bars/padding).
    work_width: i32,
    /// Y offset from top (accounting for bar height).
    yoffset: i32,
    /// Client rectangle. Only the size is used for initial/target positions.
    client_rect: Rect,
}

impl EdgePositionInfo {
    fn initial_rect(self) -> Rect {
        match self.direction {
            EdgeDirection::Top => Rect {
                x: self.monitor_rect.x + EDGE_MARGIN_X,
                y: self.monitor_rect.y + self.yoffset - self.client_rect.h,
                w: self.work_width - EDGE_INSET_X,
                h: self.client_rect.h,
            },
            EdgeDirection::Right => Rect {
                x: self.monitor_rect.x + self.monitor_rect.w - EDGE_MARGIN_X,
                y: self.monitor_rect.y + EDGE_MARGIN_Y,
                w: self.client_rect.w,
                h: self.monitor_rect.h - EDGE_INSET_Y,
            },
            EdgeDirection::Bottom => Rect {
                x: self.monitor_rect.x + EDGE_MARGIN_X,
                y: self.monitor_rect.y + self.monitor_rect.h,
                w: self.work_width - EDGE_INSET_X,
                h: self.client_rect.h,
            },
            EdgeDirection::Left => Rect {
                x: self.monitor_rect.x - self.client_rect.w + EDGE_MARGIN_X,
                y: self.monitor_rect.y + EDGE_MARGIN_Y,
                w: self.client_rect.w,
                h: self.monitor_rect.h - EDGE_INSET_Y,
            },
        }
    }

    fn target_rect(self) -> Rect {
        match self.direction {
            EdgeDirection::Top => Rect {
                x: self.monitor_rect.x + EDGE_MARGIN_X,
                y: self.monitor_rect.y + self.yoffset,
                w: self.work_width - EDGE_INSET_X,
                h: self.client_rect.h,
            },
            EdgeDirection::Right => Rect {
                x: self.monitor_rect.x + self.monitor_rect.w - self.client_rect.w,
                y: self.monitor_rect.y + EDGE_MARGIN_Y,
                w: self.client_rect.w,
                h: self.monitor_rect.h - EDGE_INSET_Y,
            },
            EdgeDirection::Bottom => Rect {
                x: self.monitor_rect.x + EDGE_MARGIN_X,
                y: self.monitor_rect.y + self.monitor_rect.h - self.client_rect.h,
                w: self.work_width - EDGE_INSET_X,
                h: self.client_rect.h,
            },
            EdgeDirection::Left => Rect {
                x: self.monitor_rect.x,
                y: self.monitor_rect.y + EDGE_MARGIN_Y,
                w: self.client_rect.w,
                h: self.monitor_rect.h - EDGE_INSET_Y,
            },
        }
    }
}

/// Positioning info for the edge slide-out (hide) animation.
#[derive(Debug, Clone, Copy)]
struct HideAnimationInfo {
    direction: EdgeDirection,
    /// Monitor rectangle (position and total size).
    monitor_rect: Rect,
    /// Current client rectangle.
    client_rect: Rect,
}

impl HideAnimationInfo {
    fn rect(self) -> Rect {
        match self.direction {
            EdgeDirection::Top => Rect {
                x: self.client_rect.x,
                y: -self.client_rect.h,
                w: 0,
                h: 0,
            },
            EdgeDirection::Right => Rect {
                x: self.monitor_rect.x + self.monitor_rect.w,
                y: self.monitor_rect.y + EDGE_MARGIN_Y,
                w: 0,
                h: 0,
            },
            EdgeDirection::Bottom => Rect {
                x: self.client_rect.x,
                y: self.monitor_rect.y + self.monitor_rect.h,
                w: 0,
                h: 0,
            },
            EdgeDirection::Left => Rect {
                x: self.monitor_rect.x - self.client_rect.w,
                y: EDGE_MARGIN_Y,
                w: 0,
                h: 0,
            },
        }
    }
}

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
    pub direction: Option<String>,
}

impl ScratchpadInfo {
    pub(crate) fn from_client(c: &crate::types::client::Client) -> Option<Self> {
        if !c.is_scratchpad() {
            return None;
        }
        Some(Self {
            name: c.scratchpad_name.clone(),
            visible: c.is_sticky,
            window_id: Some(c.win.0),
            monitor: Some(c.monitor_id.index()),
            x: Some(c.geo.x),
            y: Some(c.geo.y),
            width: Some(c.geo.w),
            height: Some(c.geo.h),
            floating: c.is_floating,
            fullscreen: c.is_fullscreen,
            direction: c.scratchpad_direction.map(|d| d.as_str().to_string()),
        })
    }
}

fn selected_or_explicit_window(ctx: &WmCtx<'_>, window_id: Option<WindowId>) -> Option<WindowId> {
    window_id.or_else(|| ctx.selected_client())
}

fn move_client_to_monitor(g: &mut Globals, win: WindowId, monitor_id: MonitorId) {
    g.detach(win);
    g.detach_z_order(win);

    if let Some(client) = g.clients.get_mut(&win) {
        client.monitor_id = monitor_id;
    }

    g.attach(win);
    g.attach_z_order_top(win);
}

fn scratchpad_names(g: &Globals, visible: bool) -> Vec<String> {
    g.clients
        .values()
        .filter(|c| c.is_scratchpad() && c.is_sticky == visible)
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
            crate::client::show_window(ctx, win);
            return true;
        }
    }
    false
}

pub fn scratchpad_make(
    ctx: &mut WmCtx,
    name: &str,
    window_id: Option<WindowId>,
    direction: Option<EdgeDirection>,
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

    // Read monitor dimensions before mutable borrow
    let (mon_ww, mon_wh) = {
        let mon = ctx.core().globals().selected_monitor();
        (mon.work_rect.w, mon.work_rect.h)
    };

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
    client.scratchpad_direction = direction;

    if !was_scratchpad {
        client.scratchpad_restore_tags = old_tags;
    }

    client.set_tag_mask(crate::types::TagMask::SCRATCHPAD);
    client.is_sticky = false;

    if !client.is_floating {
        client.is_floating = true;
    }

    if let Some(dir) = direction {
        if dir.is_vertical() {
            client.geo.h = mon_wh / 3;
        } else {
            client.geo.w = mon_ww / 3;
        }
        save_border_width(client);
        client.border_width = 0;
        client.is_locked = true;
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
    let had_direction = client.scratchpad_direction.is_some();

    let mut was_hidden = false;
    if let Some(client) = ctx.client_mut(selected_window) {
        was_hidden = client.is_hidden;
        client.set_tag_mask(if !restore_tags.is_empty() {
            restore_tags
        } else {
            monitor_tags
        });

        if had_direction {
            client.border_width = client.old_border_width;
            client.is_locked = false;
            client.scratchpad_direction = None;
        }
    }

    if was_hidden {
        crate::client::show_window(ctx, selected_window);
    } else {
        arrange(ctx, Some(monitor_id));
    }
}

pub fn scratchpad_show_name(ctx: &mut WmCtx, name: &str) -> Result<String, String> {
    let Some(found) = scratchpad_find(ctx.core().globals(), name) else {
        return Err(format!("scratchpad '{}' not found", name));
    };

    let (was_sticky, direction, target_mon) = ctx
        .core()
        .globals()
        .clients
        .get(&found)
        .map(|c| (c.is_sticky, c.scratchpad_direction, c.monitor_id))
        .unwrap_or_default();

    if was_sticky {
        return Ok(format!("scratchpad '{}' is already visible", name));
    }

    let current_mon = ctx.core().globals().selected_monitor_id();

    if let Some(client) = ctx.client_mut(found) {
        client.is_sticky = true;
        client.is_floating = true;
    }

    if target_mon != current_mon {
        move_client_to_monitor(ctx.core_mut().globals_mut(), found, current_mon);
    }

    let focusfollowsmouse = ctx.core().globals().behavior.focus_follows_mouse;

    if let Some(dir) = direction {
        ctx.core_mut().globals_mut().detach(found);
        ctx.core_mut().globals_mut().detach_z_order(found);

        if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&found) {
            client.monitor_id = current_mon;
        }

        ctx.core_mut().globals_mut().attach(found);
        ctx.core_mut().globals_mut().attach_z_order_top(found);

        let tags = ctx
            .core_mut()
            .globals_mut()
            .selected_monitor()
            .selected_tags();

        if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&found) {
            client.border_width = 0;
            client.set_tag_mask(tags);
        }

        let yoffset = {
            let mon = ctx.core().globals().selected_monitor();
            let showbar = mon.showbar_for_mask(tags);
            let bar_height = ctx.core().globals().cfg.bar_height;
            let mut offset = if showbar { bar_height } else { 0 };
            for (_win, c) in mon.iter_clients(ctx.core().globals().clients.map()) {
                if c.tags.intersects(tags) && c.is_true_fullscreen() {
                    offset = 0;
                    break;
                }
            }
            offset
        };

        let (mon_rect, mon_ww, client_rect) = {
            let mon = ctx.core().globals().monitor(current_mon).unwrap();
            let client = ctx.client(found).unwrap();
            (mon.monitor_rect, mon.work_rect.w, client.geo)
        };

        let pos_info = EdgePositionInfo {
            direction: dir,
            monitor_rect: mon_rect,
            work_width: mon_ww,
            yoffset,
            client_rect,
        };

        let initial_rect = pos_info.initial_rect();
        ctx.move_resize(
            found,
            initial_rect,
            MoveResizeOptions::hinted_immediate(true),
        );

        ctx.backend().raise_window_visual_only(found);
        let target_rect = pos_info.target_rect();
        ctx.move_resize(
            found,
            target_rect,
            MoveResizeOptions::animate_to(EMPHASIZED_FRAME_COUNT),
        );
    } else {
        let is_hidden = ctx
            .core()
            .globals()
            .clients
            .get(&found)
            .map(|c| c.is_hidden)
            .unwrap_or(false);
        if is_hidden {
            crate::client::show_window(ctx, found);
        } else {
            let mid = ctx.core().globals().selected_monitor_id();
            crate::focus::focus_soft(ctx, Some(found));
            arrange(ctx, Some(mid));
            crate::layouts::sync_monitor_z_order(ctx, mid);
        }
    }

    crate::focus::focus_soft(ctx, Some(found));
    ctx.backend().raise_window_visual_only(found);

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
            .any(|c| c.is_scratchpad() && c.scratchpad_name == name && c.is_sticky);
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

    let direction = ctx
        .core()
        .globals()
        .clients
        .get(&found)
        .and_then(|c| c.scratchpad_direction);

    let (geo, mon_rect) = {
        let mon = ctx.core().globals().selected_monitor();
        let Some(client) = ctx.client(found) else {
            return;
        };
        if !client.is_sticky {
            return;
        }
        (client.geo, mon.monitor_rect)
    };

    if let Some(client) = ctx.client_mut(found) {
        client.is_sticky = false;
        client.set_tag_mask(crate::types::TagMask::SCRATCHPAD);
    }

    if let Some(dir) = direction {
        let hide_info = HideAnimationInfo {
            direction: dir,
            monitor_rect: mon_rect,
            client_rect: geo,
        };

        let hide_rect = hide_info.rect();
        ctx.move_resize(
            found,
            hide_rect,
            MoveResizeOptions::animate_to(EMPHASIZED_FRAME_COUNT),
        );
    }

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
    let is_sticky = client.is_sticky;

    if is_sticky {
        scratchpad_hide_name(ctx, name);
    } else {
        let _ = scratchpad_show_name(ctx, name);
    }
}

pub fn collect_scratchpad_info(g: &Globals) -> Vec<ScratchpadInfo> {
    g.clients
        .values()
        .filter_map(|c| ScratchpadInfo::from_client(c))
        .collect()
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

pub fn set_scratchpad_direction(ctx: &mut WmCtx, win: WindowId, direction: EdgeDirection) {
    let was_sticky = ctx.client(win).is_some_and(|c| c.is_sticky);

    let (mon_ww, mon_wh) = {
        let mon = ctx.core().globals().selected_monitor();
        (mon.work_rect.w, mon.work_rect.h)
    };

    if let Some(client) = ctx.client_mut(win) {
        client.scratchpad_direction = Some(direction);
        if direction.is_vertical() {
            client.geo.h = mon_wh / 3;
        } else {
            client.geo.w = mon_ww / 3;
        }
    }

    if was_sticky {
        let name = ctx
            .client(win)
            .map(|c| c.scratchpad_name.clone())
            .unwrap_or_default();
        if !name.is_empty() {
            scratchpad_hide_name(ctx, &name);
            let _ = scratchpad_show_name(ctx, &name);
        }
    }
}

pub fn overlay_create(ctx: &mut WmCtx) {
    if let Some(existing) = scratchpad_find(ctx.core().globals(), OVERLAY_NAME) {
        scratchpad_unmake(ctx, Some(existing));
    }

    let Some(selected) = ctx.selected_client() else {
        return;
    };

    let is_fullscreen = ctx.client(selected).is_some_and(|c| c.is_true_fullscreen());
    if is_fullscreen {
        crate::floating::toggle_maximized(ctx);
    }

    scratchpad_make(
        ctx,
        OVERLAY_NAME,
        None,
        Some(EdgeDirection::Top),
        ScratchpadInitialStatus::Shown,
    );
}

pub fn overlay_toggle(ctx: &mut WmCtx) {
    scratchpad_toggle(ctx, Some(OVERLAY_NAME));
}
