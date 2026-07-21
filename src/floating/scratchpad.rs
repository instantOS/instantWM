use crate::constants::animation::EMPHASIZED_FRAME_COUNT;
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::ipc_types::ScratchpadInitialStatus;
use crate::layouts::arrange;
use crate::model::WmModel;
use crate::types::input::EdgeDirection;
use crate::types::{MonitorId, Rect, Size, TagMask, WindowId};
use bincode::{Decode, Encode};

const EDGE_MARGIN_X: i32 = 20;
const EDGE_MARGIN_Y: i32 = 40;

pub const DEFAULT_EDGE_SCRATCHPAD_NAME: &str = "instantwm_edge_scratchpad";
pub(crate) const SCRATCHPAD_IDENTITY_PREFIX: &str = "scratchpad_";

/// Infer the scratchpad role advertised by launchers such as `ins` before the
/// client participates in its first layout. Native Wayland uses `class` as the
/// app-id; X11 terminals may put the same identity in either WM_CLASS field.
pub(crate) fn name_from_window_identity<'a>(class: &'a str, instance: &'a str) -> Option<&'a str> {
    [class, instance].into_iter().find_map(|identity| {
        identity
            .strip_prefix(SCRATCHPAD_IDENTITY_PREFIX)
            .filter(|name| !name.is_empty())
    })
}

/// Complete geometry for one edge-scratchpad transition.
///
/// `shown` is always contained by the monitor's visible content rectangle;
/// `hidden` is immediately outside the same edge. Keeping the pair together
/// prevents show and hide paths from drifting into different policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EdgeSlideRects {
    hidden: Rect,
    shown: Rect,
}

impl EdgeSlideRects {
    fn new(content: Rect, direction: EdgeDirection, requested_size: Size) -> Self {
        let content = Rect::new(content.x, content.y, content.w.max(1), content.h.max(1));
        let horizontal_margin = EDGE_MARGIN_X.min((content.w - 1) / 2);
        let vertical_margin = EDGE_MARGIN_Y.min((content.h - 1) / 2);
        let horizontal_span = (content.w - 2 * horizontal_margin).max(1);
        let vertical_span = (content.h - 2 * vertical_margin).max(1);
        let requested_width = requested_size.w.max(1).min(content.w);
        let requested_height = requested_size.h.max(1).min(content.h);

        let shown = match direction {
            EdgeDirection::Top => Rect::new(
                content.x + horizontal_margin,
                content.y,
                horizontal_span,
                requested_height,
            ),
            EdgeDirection::Right => Rect::new(
                content.x + content.w - requested_width,
                content.y + vertical_margin,
                requested_width,
                vertical_span,
            ),
            EdgeDirection::Bottom => Rect::new(
                content.x + horizontal_margin,
                content.y + content.h - requested_height,
                horizontal_span,
                requested_height,
            ),
            EdgeDirection::Left => Rect::new(
                content.x,
                content.y + vertical_margin,
                requested_width,
                vertical_span,
            ),
        };

        let hidden = match direction {
            EdgeDirection::Top => Rect::new(shown.x, content.y - shown.h, shown.w, shown.h),
            EdgeDirection::Right => Rect::new(content.x + content.w, shown.y, shown.w, shown.h),
            EdgeDirection::Bottom => Rect::new(shown.x, content.y + content.h, shown.w, shown.h),
            EdgeDirection::Left => Rect::new(content.x - shown.w, shown.y, shown.w, shown.h),
        };

        Self { hidden, shown }
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

fn prepare_scratchpad_for_show(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    monitor_id: MonitorId,
    direction: Option<EdgeDirection>,
) {
    attach_client_to_monitor_top(ctx.core_mut().model_mut(), win, monitor_id);

    let tags = ctx
        .core()
        .model()
        .monitor(monitor_id)
        .map(|monitor| monitor.selected_tags())
        .unwrap_or(TagMask::EMPTY);
    if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
        client.show_as_scratchpad(tags, direction);
    }
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
        let mon = ctx.core().model().expect_selected_monitor();
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

    let monitor_tags = ctx.core().model().expect_selected_monitor().selected_tags();

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

#[derive(Debug, Clone, Copy)]
pub(crate) struct ScratchpadShowOptions {
    pub monitor_id: MonitorId,
    pub focus: bool,
    pub warp_pointer: bool,
}

pub fn scratchpad_show_name(ctx: &mut WmCtx, name: &str) -> Result<String, String> {
    let options = ScratchpadShowOptions {
        monitor_id: ctx.core().model().selected_monitor_id(),
        focus: true,
        warp_pointer: ctx.core().behavior().focus_follows_mouse,
    };
    scratchpad_show_name_with_options(ctx, name, options)
}

pub(crate) fn scratchpad_show_name_with_options(
    ctx: &mut WmCtx,
    name: &str,
    options: ScratchpadShowOptions,
) -> Result<String, String> {
    if ctx.core().model().monitor(options.monitor_id).is_none() {
        return Err(format!(
            "target monitor {:?} does not exist",
            options.monitor_id
        ));
    }
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

    let target_monitor = options.monitor_id;
    prepare_scratchpad_for_show(ctx, found, target_monitor, direction);

    if let Some(dir) = direction {
        let (content_rect, client_size) = {
            if !ctx.window_backend().window_exists(found) {
                return Err(format!("scratchpad '{}' no longer exists", name));
            }
            let mon = ctx
                .core()
                .model()
                .monitor(target_monitor)
                .expect("validated target monitor must exist while showing scratchpad");
            let client = ctx
                .core()
                .model()
                .client(found)
                .expect("scratchpad client must exist after window_exists check");
            (
                mon.visible_content_rect(&ctx.core().model().clients),
                client.geo.size(),
            )
        };

        let slide = EdgeSlideRects::new(content_rect, dir, client_size);

        ctx.move_resize(found, slide.hidden, MoveResizeOptions::immediate());

        reveal_scratchpad_window(ctx, found);
        ctx.move_resize(
            found,
            slide.shown,
            MoveResizeOptions::animate_to(EMPHASIZED_FRAME_COUNT),
        );
    } else {
        let was_hidden = reveal_scratchpad_window(ctx, found);
        arrange_visible_scratchpad(ctx, found, was_hidden);
    }

    if options.focus {
        crate::focus::focus(ctx, Some(found));
    }
    ctx.window_backend().raise_window_visual_only(found);

    if options.warp_pointer {
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

    let slide = {
        let Some(client) = ctx.core().model().client(found) else {
            return;
        };
        if !client.is_sticky {
            return;
        }
        let Some(mon) = ctx.core().model().monitor(client.monitor_id) else {
            return;
        };
        direction.map(|direction| {
            EdgeSlideRects::new(
                mon.visible_content_rect(&ctx.core().model().clients),
                direction,
                client.geo.size(),
            )
        })
    };

    if let Some(client) = ctx.core_mut().model_mut().client_mut(found) {
        client.hide_as_scratchpad();
    }

    if let Some(slide) = slide {
        ctx.move_resize(
            found,
            slide.hidden,
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

/// Toggle a scratchpad on a specific monitor without moving the pointer.
///
/// This is used by pointer-edge triggers: warping from inside a hot corner
/// would leave its hysteresis zone and make the interaction unstable.
pub(crate) fn scratchpad_toggle_from_hot_corner(
    ctx: &mut WmCtx,
    name: &str,
    monitor_id: MonitorId,
) {
    if ctx.core().model().is_overview_active() {
        return;
    }
    let Some(found) = ctx.core().model().scratchpad_find(name) else {
        return;
    };
    let Some(is_visible) = ctx
        .core()
        .model()
        .client(found)
        .map(|client| client.is_sticky)
    else {
        return;
    };

    if is_visible {
        scratchpad_hide_name(ctx, name);
    } else {
        // Focusing is monitor-relative in the WM model. Make the corner's
        // monitor current before attaching and focusing the scratchpad there.
        crate::focus::select_monitor(ctx, monitor_id);
        let _ = scratchpad_show_name_with_options(
            ctx,
            name,
            ScratchpadShowOptions {
                monitor_id,
                focus: true,
                warp_pointer: false,
            },
        );
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
        let mon = ctx.core().model().expect_selected_monitor();
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
        crate::floating::toggle_client_maximized(ctx);
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
    use super::{EdgeSlideRects, name_from_window_identity};
    use crate::types::input::EdgeDirection;
    use crate::types::{Rect, Size};

    fn contains(outer: Rect, inner: Rect) -> bool {
        inner.x >= outer.x
            && inner.y >= outer.y
            && inner.x + inner.w <= outer.x + outer.w
            && inner.y + inner.h <= outer.y + outer.h
    }

    #[test]
    fn scratchpad_identity_accepts_wayland_app_id_and_x11_instance() {
        assert_eq!(
            name_from_window_identity("scratchpad_menu", ""),
            Some("menu")
        );
        assert_eq!(
            name_from_window_identity("kitty", "scratchpad_notes"),
            Some("notes")
        );
        assert_eq!(name_from_window_identity("scratchpad_", "kitty"), None);
        assert_eq!(name_from_window_identity("kitty", "kitty"), None);
    }

    #[test]
    fn shown_rects_stay_inside_content_and_hidden_rects_stay_outside() {
        let content = Rect::new(100, 230, 1920, 1050);

        for direction in [
            EdgeDirection::Top,
            EdgeDirection::Right,
            EdgeDirection::Bottom,
            EdgeDirection::Left,
        ] {
            let slide = EdgeSlideRects::new(content, direction, Size::new(640, 360));

            assert!(contains(content, slide.shown), "{direction:?}");
            assert!(!content.intersects_other(&slide.hidden), "{direction:?}");
            assert_eq!(slide.hidden.size(), slide.shown.size());
        }
    }

    #[test]
    fn oversized_edge_scratchpads_are_clamped_to_content() {
        let content = Rect::new(10, 20, 8, 3);

        for direction in [
            EdgeDirection::Top,
            EdgeDirection::Right,
            EdgeDirection::Bottom,
            EdgeDirection::Left,
        ] {
            let slide = EdgeSlideRects::new(content, direction, Size::new(500, 500));

            assert!(contains(content, slide.shown), "{direction:?}");
            assert!(slide.shown.w > 0);
            assert!(slide.shown.h > 0);
        }
    }
}
