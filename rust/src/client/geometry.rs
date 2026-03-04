//! Client geometry: resizing, size-hint enforcement, and dimension helpers.
//!
//! # Responsibilities
//!
//! * [`resize`] – high-level resize that runs size-hint validation first.
//! * [`resize_client`] – low-level X11 configure + state update.
//! * [`apply_size_hints`] – clamp a proposed geometry to ICCCM `WM_NORMAL_HINTS`.
//! * [`update_size_hints`] / [`update_size_hints_win`] – read `WM_NORMAL_HINTS` from X.
//! * [`scale_client`] – resize a client to a percentage of its monitor.
//!
//! # Dimension helpers
//!
//! Client dimensions including borders are available as methods:
//! * [`Client::total_width`](crate::types::Client::total_width) – total width including borders
//! * [`Client::total_height`](crate::types::Client::total_height) – total height including borders

use crate::backend::{BackendKind, BackendOps};
use crate::client::constants::{
    SIZE_HINTS_P_ASPECT, SIZE_HINTS_P_BASE_SIZE, SIZE_HINTS_P_MAX_SIZE, SIZE_HINTS_P_MIN_SIZE,
    SIZE_HINTS_P_RESIZE_INC,
};
use crate::contexts::WmCtx;
use crate::types::{MonitorId, Rect, WindowId};

use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;

// ---------------------------------------------------------------------------
// High-level resize (validates size hints)
// ---------------------------------------------------------------------------

/// Resize `win` to the given `rect`, enforcing `WM_NORMAL_HINTS` constraints.
///
/// If the size-hint check determines that nothing changed *and* there is more
/// than one client on screen, the X11 configure call is skipped.  With a
/// single client we always apply the resize so the window fills its space.
pub fn resize(ctx: &mut WmCtx, win: WindowId, rect: &Rect, interact: bool) {
    if !ctx.g.clients.contains(&win) {
        return;
    }

    let mut new_x = rect.x;
    let mut new_y = rect.y;
    let mut new_width = rect.w;
    let mut new_height = rect.h;
    let changed = apply_size_hints(
        ctx,
        win,
        &mut new_x,
        &mut new_y,
        &mut new_width,
        &mut new_height,
        interact,
    );
    let client_count = ctx.g.clients.len();
    if changed || client_count == 1 {
        resize_client(
            ctx,
            win,
            &Rect {
                x: new_x,
                y: new_y,
                w: new_width,
                h: new_height,
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Low-level resize (direct X11 configure)
// ---------------------------------------------------------------------------

/// Update the stored geometry for `win` and issue an X11 `ConfigureWindow`.
///
/// This is the single source of truth for moving/resizing a window at the X11
/// level.  Always call [`resize`] from layout code so that size hints are
/// respected; call this directly only when you have already validated the
/// geometry (e.g. during fullscreen transitions).
pub fn resize_client(ctx: &mut WmCtx, win: WindowId, rect: &Rect) {
    ctx.g.clients.update_geometry(win, *rect);

    ctx.backend.resize_window(win, *rect);

    if ctx.backend_kind() == BackendKind::X11 {
        if let Some(x11) = ctx.x11_conn() {
            let x11_win: Window = win.into();
            if let Some(border_width) = ctx.g.clients.get(&win).map(|c| c.border_width) {
                let _ = x11.conn.configure_window(
                    x11_win,
                    &ConfigureWindowAux::new().border_width(border_width as u32),
                );
            }
        }
    }

    // Send a synthetic ConfigureNotify so the client knows its geometry.
    crate::client::focus::configure(ctx, win);
}

// ---------------------------------------------------------------------------
// Size-hint enforcement (ICCCM §4.1.2.3)
// ---------------------------------------------------------------------------

/// Clamp and snap `(x, y, w, h)` to the client's `WM_NORMAL_HINTS`.
///
/// When `interact` is `true` the bounds are the full screen dimensions;
/// otherwise the client's monitor work-area is used.
///
/// Returns `true` if the resulting geometry differs from the client's current
/// stored geometry (i.e. an actual change would occur).
pub fn apply_size_hints(
    ctx: &mut WmCtx,
    win: WindowId,
    x: &mut i32,
    y: &mut i32,
    w: &mut i32,
    h: &mut i32,
    interact: bool,
) -> bool {
    let client = match ctx.g.clients.get(&win) {
        Some(c) => c,
        None => return false,
    };

    let old_geo = client.geo;
    let mut new_geo = Rect::new(*x, *y, *w, *h);
    let border_width = client.border_width;
    let monitor_id = client.monitor_id;
    let should_apply_hints =
        ctx.g.cfg.resizehints != 0 || client.isfloating || is_floating_layout(ctx, monitor_id);

    // Phase 1: Ensure positive dimensions.
    new_geo.w = new_geo.w.max(1);
    new_geo.h = new_geo.h.max(1);

    // Phase 2: Clamp position to keep window visible.
    clamp_position_to_bounds(
        ctx,
        &mut new_geo,
        monitor_id,
        interact,
        old_geo.total_width(border_width),
        old_geo.total_height(border_width),
    );

    // Phase 3: Enforce minimum size (bar height).
    let bar_height = ctx.g.cfg.bar_height;
    new_geo.enforce_minimum(bar_height, bar_height);

    // Phase 4: Apply ICCCM size hints (X11 only).
    if ctx.backend_kind() == BackendKind::X11 && should_apply_hints {
        apply_icccm_size_hints(ctx, win, &mut new_geo);
    }

    // Write back results.
    *x = new_geo.x;
    *y = new_geo.y;
    *w = new_geo.w;
    *h = new_geo.h;

    new_geo.differs_from(&old_geo)
}

/// Clamp window position to keep it within usable screen area.
fn clamp_position_to_bounds(
    ctx: &WmCtx,
    geo: &mut Rect,
    monitor_id: Option<MonitorId>,
    interact: bool,
    total_w: i32,
    total_h: i32,
) {
    if interact {
        let screen = Rect::new(0, 0, ctx.g.cfg.screen_width, ctx.g.cfg.screen_height);
        geo.clamp_position(&screen, total_w, total_h);
    } else if let Some(wr) = monitor_id
        .and_then(|mid| ctx.g.monitors.get(mid))
        .map(|m| m.work_rect)
    {
        geo.clamp_position(&wr, total_w, total_h);
    }
}

/// Check if the client's monitor is using a floating layout.
fn is_floating_layout(ctx: &WmCtx, monitor_id: Option<MonitorId>) -> bool {
    monitor_id
        .and_then(|mid| ctx.g.monitors.get(mid))
        .map(|mon| !mon.is_tiling_layout())
        .unwrap_or(true)
}

/// Apply ICCCM WM_NORMAL_HINTS constraints to the geometry.
fn apply_icccm_size_hints(ctx: &mut WmCtx, win: WindowId, geo: &mut Rect) {
    let needs_update = ctx
        .g
        .clients
        .get(&win)
        .map(|c| c.hintsvalid == 0)
        .unwrap_or(false);

    if needs_update {
        update_size_hints(ctx, win);
    }

    let client = match ctx.g.clients.get(&win) {
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

// -----------------------------------------------------------------------------
// WM_NORMAL_HINTS parsing
// ---------------------------------------------------------------------------

/// Read `WM_NORMAL_HINTS` from the X server and populate the client's size hints,
/// `min_aspect`, `max_aspect`, and `isfixed`.
pub fn update_size_hints(ctx: &mut WmCtx, win: WindowId) {
    let Some(data) = fetch_wm_normal_hints(ctx, win) else {
        return;
    };
    let Some(c) = ctx.g.clients.get_mut(&win) else {
        return;
    };
    let flags = *data.first().unwrap_or(&0);
    let at = |idx: usize| -> i32 { data.get(idx).copied().unwrap_or(0) as i32 };

    // Base size (idx 15-16), fallback to min size (idx 5-6)
    (c.size_hints.basew, c.size_hints.baseh) =
        if flags & SIZE_HINTS_P_BASE_SIZE != 0 && data.len() > 16 {
            (at(15), at(16))
        } else if flags & SIZE_HINTS_P_MIN_SIZE != 0 && data.len() > 6 {
            (at(5), at(6))
        } else {
            (0, 0)
        };

    // Min size (idx 5-6), fallback to base size
    (c.size_hints.minw, c.size_hints.minh) = if flags & SIZE_HINTS_P_MIN_SIZE != 0 && data.len() > 6
    {
        (at(5), at(6))
    } else if flags & SIZE_HINTS_P_BASE_SIZE != 0 {
        (c.size_hints.basew, c.size_hints.baseh)
    } else {
        (0, 0)
    };

    // Max size (idx 7-8)
    (c.size_hints.maxw, c.size_hints.maxh) = if flags & SIZE_HINTS_P_MAX_SIZE != 0 && data.len() > 8
    {
        (at(7), at(8))
    } else {
        (0, 0)
    };

    // Resize increments (idx 9-10)
    (c.size_hints.incw, c.size_hints.inch) =
        if flags & SIZE_HINTS_P_RESIZE_INC != 0 && data.len() > 10 {
            (at(9), at(10))
        } else {
            (0, 0)
        };

    // Aspect ratios (idx 11-14)
    (c.min_aspect, c.max_aspect) = if flags & SIZE_HINTS_P_ASPECT != 0 && data.len() > 14 {
        let min_d = at(12);
        let max_d = at(14);
        (
            if min_d != 0 {
                at(11) as f32 / min_d as f32
            } else {
                0.0
            },
            if max_d != 0 {
                at(13) as f32 / max_d as f32
            } else {
                0.0
            },
        )
    } else {
        (0.0, 0.0)
    };

    c.isfixed = c.size_hints.is_fixed();
    c.hintsvalid = 1;
}

fn fetch_wm_normal_hints(ctx: &mut WmCtx, win: WindowId) -> Option<Vec<u32>> {
    let conn = ctx.x11_conn().map(|x11| x11.conn)?;
    let reply = conn
        .get_property(
            false,
            win.into(),
            AtomEnum::WM_NORMAL_HINTS,
            AtomEnum::WM_SIZE_HINTS,
            0,
            24,
        )
        .ok()?
        .reply()
        .ok()?;
    reply.value32().map(|v| v.collect())
}

// ---------------------------------------------------------------------------
// Scale helper
// ---------------------------------------------------------------------------

/// Resize `win` to `scale` percent of its monitor dimensions, centred on screen.
///
/// `scale` is an integer percentage (e.g. `75` means 75 %).
pub fn scale_client(ctx: &mut WmCtx, win: WindowId, scale: i32) {
    let (monitor_id, old_geo, border_width) = {
        let c = match ctx.g.clients.get(&win) {
            Some(c) => c,
            None => return,
        };
        (c.monitor_id, c.geo, c.border_width)
    };

    // Determine the reference rectangle (monitor bounds, or fall back to the
    // client's own geometry when no monitor is assigned).
    let mon_rect = monitor_id
        .and_then(|mid| ctx.g.monitors.get(mid).map(|m| m.monitor_rect))
        .unwrap_or(old_geo);

    let new_w = old_geo.w * scale / 100;
    let new_h = old_geo.h * scale / 100;
    let new_x = mon_rect.x + (mon_rect.w - new_w) / 2 - border_width;
    let new_y = mon_rect.y + (mon_rect.h - new_h) / 2 - border_width;

    resize(
        ctx,
        win,
        &Rect {
            x: new_x,
            y: new_y,
            w: new_w,
            h: new_h,
        },
        false,
    );
}
