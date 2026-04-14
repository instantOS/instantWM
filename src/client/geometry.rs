//! Client geometry: resizing, size-hint enforcement, and dimension helpers.
//!
//! # Responsibilities
//!
//! * [`resize`] – high-level resize that runs size-hint validation first.
//! * [`WmCtx::move_resize`](crate::contexts::WmCtx::move_resize) – low-level configure + state update.
//! * [`apply_size_hints`] – clamp a proposed geometry to ICCCM size hints.
//! * [`scale_client`] – resize a client to a percentage of its monitor.
//!
//! # Dimension helpers
//!
//! Client dimensions including borders are available as methods:
//! * [`Client::total_width`](crate::types::Client::total_width) – total width including borders
//! * [`Client::total_height`](crate::types::Client::total_height) – total height including borders

use crate::backend::x11::X11BackendRef;
use crate::contexts::CoreCtx;
use crate::geometry::MoveResizeOptions;
use crate::globals::Globals;
use crate::types::{Rect, WindowId};

/// Record the resolved geometry of a managed client.
///
/// Backends may request a resize optimistically, but this helper is called only
/// once the WM knows the geometry that actually applies to the window right
/// now. Shared state lives here so backend callbacks do not each reinvent the
/// `geo` / `old_geo` / `float_geo` update contract.
pub fn sync_client_geometry(globals: &mut Globals, win: WindowId, rect: Rect) {
    let Some(client) = globals.clients.get_mut(&win) else {
        return;
    };
    client.old_geo = client.geo;
    client.geo = rect;
    if client.is_floating {
        client.float_geo = rect;
    }
}

/// Compute a saner initial position for a newly managed floating client.
///
/// The goal is to preserve application-provided placement when it is already
/// reasonable, while preventing new floats from spawning under the bar or
/// mostly off-screen. The returned rect keeps the original size and only
/// adjusts position.
pub fn sane_floating_spawn_rect(globals: &Globals, win: WindowId) -> Option<Rect> {
    let client = globals.clients.get(&win)?;
    if !client.is_floating {
        return None;
    }

    let work_rect = client.monitor(globals)?.work_rect;
    if !work_rect.is_valid() {
        return None;
    }

    let total_w = client.total_width();
    let total_h = client.total_height();
    let mut rect = client.geo;

    let fully_outside_x = rect.x + total_w <= work_rect.x || rect.x >= work_rect.x + work_rect.w;
    let fully_outside_y = rect.y + total_h <= work_rect.y || rect.y >= work_rect.y + work_rect.h;

    rect.x = normalize_spawn_axis(rect.x, total_w, work_rect.x, work_rect.w, fully_outside_x);
    rect.y = normalize_spawn_axis(rect.y, total_h, work_rect.y, work_rect.h, fully_outside_y);

    rect.differs_from(&client.geo).then_some(rect)
}

fn normalize_spawn_axis(
    pos: i32,
    total_len: i32,
    bounds_pos: i32,
    bounds_len: i32,
    fully_outside: bool,
) -> i32 {
    if total_len >= bounds_len {
        return bounds_pos;
    }

    let min_pos = bounds_pos;
    let max_pos = bounds_pos + bounds_len - total_len;

    if fully_outside {
        bounds_pos + (bounds_len - total_len) / 2
    } else {
        pos.clamp(min_pos, max_pos)
    }
}

// ---------------------------------------------------------------------------
// High-level resize (validates size hints)
// ---------------------------------------------------------------------------

/// Backend-agnostic resize entry point.
///
pub fn resize(ctx: &mut crate::contexts::WmCtx<'_>, win: WindowId, rect: &Rect, interact: bool) {
    let mut new_rect = *rect;

    let changed = match ctx {
        crate::contexts::WmCtx::X11(x11_ctx) => apply_size_hints(
            &mut x11_ctx.core,
            Some(&x11_ctx.x11),
            win,
            &mut new_rect,
            interact,
        ),
        crate::contexts::WmCtx::Wayland(wl_ctx) => {
            apply_size_hints(&mut wl_ctx.core, None, win, &mut new_rect, interact)
        }
    };
    let client_count = ctx.core().globals().clients.len();

    if changed || client_count == 1 {
        ctx.move_resize(win, new_rect, MoveResizeOptions::immediate());
    }
}

/// Apply size hints to the given rect and return whether it changed.
///
/// Returns `true` if the resulting geometry differs from the client's current
/// stored geometry (i.e. an actual change would occur).
pub fn apply_size_hints(
    core: &mut CoreCtx,
    x11: Option<&X11BackendRef>,
    win: WindowId,
    rect: &mut Rect,
    interact: bool,
) -> bool {
    let client = match core.client(win) {
        Some(c) => c,
        None => return false,
    };

    let old_geo = client.geo;
    let border_width = client.border_width;
    let monitor_id = client.monitor_id;
    let should_apply_hints = core.globals().cfg.resizehints != 0
        || client.is_floating
        || is_floating_layout(core, monitor_id);

    // Phase 1: Ensure positive dimensions.
    rect.w = rect.w.max(1);
    rect.h = rect.h.max(1);

    // Phase 2: Clamp position to keep window visible.
    clamp_position_to_bounds(
        core,
        rect,
        monitor_id,
        interact,
        old_geo.total_width(border_width),
        old_geo.total_height(border_width),
    );

    // Phase 3: Enforce minimum size (bar height).
    let bar_height = core.globals().cfg.bar_height;
    rect.enforce_minimum(bar_height, bar_height);

    // Phase 4: Apply ICCCM size hints (X11 only).
    if should_apply_hints && let Some(x11_backend) = x11 {
        apply_icccm_size_hints_x11(core, x11_backend, win, rect);
    }

    rect.differs_from(&old_geo)
}

/// Clamp window position to keep it within usable screen area.
fn clamp_position_to_bounds(
    core: &CoreCtx,
    geo: &mut Rect,
    monitor_id: crate::types::MonitorId,
    interact: bool,
    total_w: i32,
    total_h: i32,
) {
    if interact {
        let screen = Rect::new(
            0,
            0,
            core.globals().cfg.screen_width,
            core.globals().cfg.screen_height,
        );
        geo.clamp_position(&screen, total_w, total_h);
    } else if let Some(wr) = core.globals().monitors.get(monitor_id).map(|m| m.work_rect) {
        geo.clamp_position(&wr, total_w, total_h);
    }
}

/// Check if the client's monitor is using a floating layout.
fn is_floating_layout(core: &CoreCtx, monitor_id: crate::types::MonitorId) -> bool {
    core.globals()
        .monitors
        .get(monitor_id)
        .map(|mon| !mon.is_tiling_layout())
        .unwrap_or(true)
}

fn apply_icccm_size_hints_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    win: WindowId,
    geo: &mut Rect,
) {
    let needs_update = core
        .client(win)
        .map(|c| c.size_hints_valid == 0)
        .unwrap_or(false);

    if needs_update {
        crate::backend::x11::client::update_size_hints_x11(core, x11, win);
    }

    let client = match core.client(win) {
        Some(c) => c,
        None => return,
    };

    let (w, h) =
        client
            .size_hints
            .constrain_size(geo.w, geo.h, client.min_aspect, client.max_aspect);
    geo.w = w;
    geo.h = h;
}

// ---------------------------------------------------------------------------
// Scale helper
// ---------------------------------------------------------------------------

/// Calculate the target rect for scaling a client to `scale` percent of its monitor.
fn calculate_scaled_geometry(
    monitor_id: crate::types::MonitorId,
    old_geo: Rect,
    border_width: i32,
    scale: i32,
    get_monitor_rect: impl FnOnce(crate::types::MonitorId) -> Rect,
) -> Rect {
    let mon_rect = get_monitor_rect(monitor_id);

    let new_w = old_geo.w * scale / 100;
    let new_h = old_geo.h * scale / 100;
    let new_x = mon_rect.x + (mon_rect.w - new_w) / 2 - border_width;
    let new_y = mon_rect.y + (mon_rect.h - new_h) / 2 - border_width;

    Rect {
        x: new_x,
        y: new_y,
        w: new_w,
        h: new_h,
    }
}

#[cfg(test)]
mod tests {
    use super::sane_floating_spawn_rect;
    use crate::globals::Globals;
    use crate::types::{Client, Monitor, MonitorId, Rect, TagMask, WindowId};

    fn globals_with_floating_client(rect: Rect, border_width: i32, work_rect: Rect) -> Globals {
        let mut globals = Globals::default();

        let mut monitor = Monitor::new_with_values(true, true);
        monitor.monitor_rect = Rect::new(work_rect.x, work_rect.y, work_rect.w, work_rect.h);
        monitor.work_rect = work_rect;
        monitor.set_selected_tags(TagMask::single(1).unwrap());
        globals.monitors.push(monitor);

        let mut client = Client::default();
        client.win = WindowId::from(1_u32);
        client.monitor_id = MonitorId(0);
        client.set_tag_mask(TagMask::single(1).unwrap());
        client.is_floating = true;
        client.border_width = border_width;
        client.geo = rect;
        client.float_geo = rect;
        client.old_geo = rect;
        globals.clients.insert(client.win, client);

        globals
    }

    #[test]
    fn sane_floating_spawn_rect_clamps_under_bar() {
        let globals = globals_with_floating_client(
            Rect::new(100, 0, 500, 300),
            2,
            Rect::new(0, 32, 1920, 1048),
        );

        let rect = sane_floating_spawn_rect(&globals, WindowId::from(1_u32)).unwrap();
        assert_eq!(rect.y, 32);
    }

    #[test]
    fn sane_floating_spawn_rect_centers_when_completely_offscreen() {
        let globals = globals_with_floating_client(
            Rect::new(-4000, -3000, 500, 300),
            2,
            Rect::new(0, 32, 1920, 1048),
        );

        let rect = sane_floating_spawn_rect(&globals, WindowId::from(1_u32)).unwrap();
        assert_eq!(rect.x, 708);
        assert_eq!(rect.y, 404);
    }

    #[test]
    fn sane_floating_spawn_rect_anchors_large_windows_to_work_area() {
        let globals = globals_with_floating_client(
            Rect::new(200, 200, 1900, 1100),
            2,
            Rect::new(0, 32, 1920, 1048),
        );

        let rect = sane_floating_spawn_rect(&globals, WindowId::from(1_u32)).unwrap();
        assert_eq!(rect.x, 16);
        assert_eq!(rect.y, 32);
    }
}

/// Resize `win` to `scale` percent of its monitor dimensions, centred on screen.
///
/// `scale` is an integer percentage (e.g. `75` means 75 %).
pub fn scale_client(ctx: &mut crate::contexts::WmCtx<'_>, win: WindowId, scale: i32) {
    let target = {
        let core = ctx.core();
        let c = match core.client(win) {
            Some(c) => c,
            None => return,
        };
        calculate_scaled_geometry(c.monitor_id, c.geo, c.border_width, scale, |mid| {
            core.globals()
                .monitors
                .get(mid)
                .map(|m| m.monitor_rect)
                .unwrap_or(c.geo)
        })
    };

    resize(ctx, win, &target, false);
}
