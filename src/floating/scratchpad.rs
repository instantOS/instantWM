use crate::constants::animation::EMPHASIZED_FRAME_COUNT;
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::ipc_types::ScratchpadInitialStatus;
use crate::layouts::arrange;
use crate::model::WmModel;
use crate::types::input::EdgeDirection;
use crate::types::{MonitorId, Rect, TagMask, WindowId};
use bincode::{Decode, Encode};

const EDGE_MARGIN_X: i32 = 20;
const EDGE_MARGIN_Y: i32 = 40;
const EDGE_INSET_X: i32 = 40;
const EDGE_INSET_Y: i32 = 80;

pub const DEFAULT_EDGE_SCRATCHPAD_NAME: &str = "instantwm_edge_scratchpad";

/// Positioning info for the edge slide-in animation.
#[derive(Debug, Clone, Copy)]
struct EdgePositionInfo {
    direction: EdgeDirection,
    /// Monitor rectangle (position and total size).
    monitor_rect: Rect,
    /// Work area width (excluding bars/padding).
    work_width: i32,
    /// Y offset from top (accounting for bar height).
    y_offset: i32,
    /// Client rectangle. Only the size is used for initial/target positions.
    client_rect: Rect,
}

impl EdgePositionInfo {
    fn initial_rect(self) -> Rect {
        match self.direction {
            EdgeDirection::Top => Rect {
                x: self.monitor_rect.x + EDGE_MARGIN_X,
                y: self.monitor_rect.y + self.y_offset - self.client_rect.h,
                w: self.work_width - EDGE_INSET_X,
                h: self.client_rect.h,
            },
            EdgeDirection::Right => Rect {
                x: self.monitor_rect.x + self.monitor_rect.w,
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
                x: self.monitor_rect.x - self.client_rect.w,
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
                y: self.monitor_rect.y + self.y_offset,
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
                y: self.monitor_rect.y - self.client_rect.h,
                w: self.client_rect.w,
                h: self.client_rect.h,
            },
            EdgeDirection::Right => Rect {
                x: self.monitor_rect.x + self.monitor_rect.w,
                y: self.monitor_rect.y + EDGE_MARGIN_Y,
                w: self.client_rect.w,
                h: self.monitor_rect.h - EDGE_INSET_Y,
            },
            EdgeDirection::Bottom => Rect {
                x: self.client_rect.x,
                y: self.monitor_rect.y + self.monitor_rect.h,
                w: self.client_rect.w,
                h: self.client_rect.h,
            },
            EdgeDirection::Left => Rect {
                x: self.monitor_rect.x - self.client_rect.w,
                y: self.monitor_rect.y + EDGE_MARGIN_Y,
                w: self.client_rect.w,
                h: self.monitor_rect.h - EDGE_INSET_Y,
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
    pub mode: crate::types::ClientMode,
    pub direction: Option<String>,
}

impl ScratchpadInfo {
    pub(crate) fn from_client(
        c: &crate::types::client::Client,
        monitor_position: usize,
    ) -> Option<Self> {
        if !c.is_scratchpad() {
            return None;
        }
        let sp = c.scratchpad.as_ref()?;
        Some(Self {
            name: sp.name.clone(),
            visible: c.is_sticky,
            window_id: Some(c.win.0),
            monitor: Some(monitor_position),
            x: Some(c.geo.x),
            y: Some(c.geo.y),
            width: Some(c.geo.w),
            height: Some(c.geo.h),
            mode: c.mode,
            direction: sp.direction.map(|d| d.as_str().to_string()),
        })
    }
}

fn selected_or_explicit_window(model: &WmModel, window_id: Option<WindowId>) -> Option<WindowId> {
    window_id.or_else(|| model.selected_win())
}

fn attach_client_to_monitor_top(model: &mut WmModel, win: WindowId, monitor_id: MonitorId) {
    model.detach(win);
    model.detach_z_order(win);

    if let Some(client) = model.client_mut(win) {
        client.monitor_id = monitor_id;
    }

    model.attach(win);
    model.attach_z_order_top(win);
}

fn selected_monitor_yoffset(model: &WmModel, bar_height: i32, tags: crate::types::TagMask) -> i32 {
    let mon = model.selected_monitor();
    let show_bar = mon.show_bar_for_mask(tags);
    let mut offset = if show_bar { bar_height } else { 0 };
    for (_win, c) in mon.iter_clients(&model.clients) {
        if c.tags.intersects(tags) && c.mode.is_true_fullscreen() {
            offset = 0;
            break;
        }
    }
    offset
}

fn prepare_scratchpad_for_show(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    monitor_id: MonitorId,
    direction: Option<EdgeDirection>,
) -> crate::types::TagMask {
    attach_client_to_monitor_top(ctx.core_mut().model_mut(), win, monitor_id);

    let tags = ctx.core().model().selected_monitor().selected_tags();
    if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
        client.show_as_scratchpad(tags, direction);
    }
    tags
}

fn reveal_scratchpad_window(ctx: &mut WmCtx<'_>, win: WindowId) -> bool {
    let was_hidden = ctx
        .core()
        .state()
        .model
        .client(win)
        .map(|c| c.is_hidden)
        .unwrap_or(false);

    if was_hidden {
        crate::client::show_window(ctx, win);
    }

    ctx.window_backend().map_window(win);
    ctx.window_backend().flush();

    was_hidden
}

fn arrange_visible_scratchpad(ctx: &mut WmCtx<'_>, win: WindowId, was_hidden: bool) {
    if was_hidden {
        return;
    }

    let Some(mid) = ctx.core().state().model.client(win).map(|c| c.monitor_id) else {
        return;
    };
    arrange(ctx, Some(mid));
    crate::layouts::sync_monitor_z_order(ctx, mid);
}

fn scratchpad_names(model: &WmModel, visible: bool) -> Vec<String> {
    model
        .clients
        .values()
        .filter(|c| c.is_scratchpad() && c.is_sticky == visible)
        .filter_map(|c| c.scratchpad.as_ref().map(|sp| sp.name.clone()))
        .collect()
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

    let target = selected_or_explicit_window(ctx.core().model(), window_id);
    let Some(selected_window) = target else {
        return;
    };

    if ctx.core().model().scratchpad_find(name).is_some() {
        return;
    }

    // Read monitor dimensions before mutable borrow
    let (mon_ww, mon_wh) = {
        let mon = ctx.core().model().selected_monitor();
        (mon.work_rect().w, mon.work_rect().h)
    };

    let Some(client) = ctx.core_mut().state_mut().model.client_mut(selected_window) else {
        return;
    };

    let was_scratchpad = client.is_scratchpad();
    let restore_tags = if was_scratchpad {
        TagMask::EMPTY
    } else {
        client.tags
    };
    client.apply_scratchpad_state(name, direction, restore_tags, mon_ww, mon_wh);

    crate::client::hide(ctx, selected_window);

    if matches!(status, ScratchpadInitialStatus::Shown) {
        let _ = scratchpad_show_name(ctx, name);
    }
}

pub fn scratchpad_unmake(ctx: &mut WmCtx, window_id: Option<WindowId>) {
    let target = selected_or_explicit_window(ctx.core().model(), window_id);
    let Some(selected_window) = target else {
        return;
    };

    let monitor_tags = ctx.core().model().selected_monitor().selected_tags();

    let Some(client) = ctx.core().model().client(selected_window) else {
        return;
    };
    if !client.is_scratchpad() {
        return;
    }
    let restore_tags = client
        .scratchpad
        .as_ref()
        .map(|sp| sp.restore_tags)
        .unwrap_or(TagMask::EMPTY);
    let monitor_id = client.monitor_id;
    let had_direction = client.is_edge_scratchpad();

    let effective_tags = if restore_tags.is_empty() {
        monitor_tags
    } else {
        restore_tags
    };

    let mut was_hidden = false;
    if let Some(client) = ctx.core_mut().state_mut().model.client_mut(selected_window) {
        was_hidden = client.is_hidden;
        client.exit_scratchpad_state(effective_tags, had_direction);
    }

    if was_hidden {
        crate::client::show_window(ctx, selected_window);
    } else {
        arrange(ctx, Some(monitor_id));
    }
}

pub fn scratchpad_show_name(ctx: &mut WmCtx, name: &str) -> Result<String, String> {
    let Some(found) = ctx.core().model().scratchpad_find(name) else {
        return Err(format!("scratchpad '{}' not found", name));
    };

    let Some((was_sticky, direction)) = ctx.core().state().model.client(found).map(|c| {
        (
            c.is_sticky,
            c.scratchpad.as_ref().and_then(|sp| sp.direction),
        )
    }) else {
        return Err(format!("scratchpad '{}' disappeared", name));
    };

    if was_sticky {
        return Ok(format!("scratchpad '{}' is already visible", name));
    }

    let current_mon = ctx.core().model().selected_monitor_id();
    let focusfollowsmouse = ctx.core().behavior().focus_follows_mouse;
    let tags = prepare_scratchpad_for_show(ctx, found, current_mon, direction);

    if let Some(dir) = direction {
        let yoffset = selected_monitor_yoffset(
            ctx.core().model(),
            ctx.core().config().derived.bar_height,
            tags,
        );
        let (mon_rect, mon_ww, client_rect) = {
            if !ctx.window_backend().window_exists(found) {
                return Err(format!("scratchpad '{}' no longer exists", name));
            }
            let mon = ctx
                .core()
                .model()
                .monitor(current_mon)
                .expect("selected monitor must exist while showing scratchpad");
            let client = ctx
                .core()
                .model()
                .client(found)
                .expect("scratchpad client must exist after window_exists check");
            (mon.monitor_rect, mon.work_rect().w, client.geo)
        };

        let pos_info = EdgePositionInfo {
            direction: dir,
            monitor_rect: mon_rect,
            work_width: mon_ww,
            y_offset: yoffset,
            client_rect,
        };

        let initial_rect = pos_info.initial_rect();
        ctx.move_resize(found, initial_rect, MoveResizeOptions::immediate());

        reveal_scratchpad_window(ctx, found);
        let target_rect = pos_info.target_rect();
        ctx.move_resize(
            found,
            target_rect,
            MoveResizeOptions::animate_to(EMPHASIZED_FRAME_COUNT),
        );
    } else {
        let was_hidden = reveal_scratchpad_window(ctx, found);
        arrange_visible_scratchpad(ctx, found, was_hidden);
    }

    crate::focus::focus(ctx, Some(found));
    ctx.window_backend().raise_window_visual_only(found);

    if focusfollowsmouse {
        ctx.warp_cursor_to_client(found);
    }

    Ok(format!("shown scratchpad '{}'", name))
}

pub fn scratchpad_show_all(ctx: &mut WmCtx) -> Option<String> {
    let scratchpad_names = scratchpad_names(ctx.core().model(), false);

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
    let scratchpad_names = scratchpad_names(ctx.core().model(), true);

    let mut hidden_count = 0;

    for name in scratchpad_names {
        let was_visible = ctx.core().model().clients.values().any(|c| {
            c.is_scratchpad()
                && c.scratchpad.as_ref().is_some_and(|sp| sp.name == name)
                && c.is_sticky
        });
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
    let Some(found) = ctx.core().model().scratchpad_find(name) else {
        return;
    };

    let direction = ctx
        .core()
        .state()
        .model
        .client(found)
        .and_then(|c| c.scratchpad.as_ref().and_then(|sp| sp.direction));

    let (geo, mon_rect) = {
        let mon = ctx.core().model().selected_monitor();
        let Some(client) = ctx.core().model().client(found) else {
            return;
        };
        if !client.is_sticky {
            return;
        }
        (client.geo, mon.monitor_rect)
    };

    if let Some(client) = ctx.core_mut().model_mut().client_mut(found) {
        client.hide_as_scratchpad();
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

    let is_overview = ctx.core().model().is_overview_active();

    if is_overview {
        return;
    }

    let found = match ctx.core().model().scratchpad_find(name) {
        Some(w) => w,
        None => return,
    };

    let Some(client) = ctx.core().model().client(found) else {
        return;
    };
    let is_sticky = client.is_sticky;

    if is_sticky {
        scratchpad_hide_name(ctx, name);
    } else {
        let _ = scratchpad_show_name(ctx, name);
    }
}

pub fn collect_scratchpad_info(model: &WmModel) -> Vec<ScratchpadInfo> {
    model
        .clients
        .values()
        .filter_map(|c| {
            let pos = model.monitors.position_of(c.monitor_id)?;
            ScratchpadInfo::from_client(c, pos)
        })
        .collect()
}

pub fn set_scratchpad_direction(ctx: &mut WmCtx, win: WindowId, direction: EdgeDirection) {
    let was_sticky = ctx
        .core()
        .state()
        .model
        .client(win)
        .is_some_and(|c| c.is_sticky);

    let (mon_ww, mon_wh) = {
        let mon = ctx.core().model().selected_monitor();
        (mon.work_rect().w, mon.work_rect().h)
    };

    if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
        if let Some(sp) = &mut client.scratchpad {
            sp.set_direction(direction);
        }
        if direction.is_vertical() {
            client.geo.h = mon_wh / 3;
        } else {
            client.geo.w = mon_ww / 3;
        }
    }

    if was_sticky {
        let Some(name) = ctx
            .core()
            .state()
            .model
            .client(win)
            .and_then(|c| c.scratchpad.as_ref().map(|sp| sp.name.clone()))
        else {
            return;
        };
        scratchpad_hide_name(ctx, &name);
        let _ = scratchpad_show_name(ctx, &name);
    }
}

pub fn edge_scratchpad_create(ctx: &mut WmCtx) {
    if let Some(existing) = ctx
        .core()
        .model()
        .scratchpad_find(DEFAULT_EDGE_SCRATCHPAD_NAME)
    {
        scratchpad_unmake(ctx, Some(existing));
    }

    let Some(selected) = ctx.core().model().selected_win() else {
        return;
    };

    let is_fullscreen = ctx
        .core()
        .state()
        .model
        .client(selected)
        .is_some_and(|c| c.mode.is_true_fullscreen());
    if is_fullscreen {
        crate::floating::toggle_maximized(ctx);
    }

    scratchpad_make(
        ctx,
        DEFAULT_EDGE_SCRATCHPAD_NAME,
        None,
        Some(EdgeDirection::Top),
        ScratchpadInitialStatus::Shown,
    );
}

#[cfg(test)]
mod tests {
    use super::{EDGE_INSET_Y, EDGE_MARGIN_Y, EdgePositionInfo, HideAnimationInfo};
    use crate::types::Rect;
    use crate::types::input::EdgeDirection;

    fn edge_info(direction: EdgeDirection) -> EdgePositionInfo {
        EdgePositionInfo {
            direction,
            monitor_rect: Rect::new(100, 200, 1920, 1080),
            work_width: 1920,
            y_offset: 30,
            client_rect: Rect::new(300, 400, 640, 360),
        }
    }

    #[test]
    fn edge_initial_rects_start_fully_offscreen_for_side_edges() {
        let right = edge_info(EdgeDirection::Right).initial_rect();
        assert_eq!(right.x, 2020);
        assert_eq!(right.w, 640);

        let left = edge_info(EdgeDirection::Left).initial_rect();
        assert_eq!(left.x + left.w, 100);
        assert_eq!(left.w, 640);
    }

    #[test]
    fn edge_hide_rects_keep_valid_size() {
        let client_rect = Rect::new(300, 400, 640, 360);
        let monitor_rect = Rect::new(100, 200, 1920, 1080);

        let top = HideAnimationInfo {
            direction: EdgeDirection::Top,
            monitor_rect,
            client_rect,
        }
        .rect();
        assert_eq!(top, Rect::new(300, -160, 640, 360));

        let left = HideAnimationInfo {
            direction: EdgeDirection::Left,
            monitor_rect,
            client_rect,
        }
        .rect();
        assert_eq!(
            left,
            Rect::new(100 - 640, 200 + EDGE_MARGIN_Y, 640, 1080 - EDGE_INSET_Y)
        );
    }
}
