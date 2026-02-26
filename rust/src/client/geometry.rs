//! Client geometry: resizing, size-hint enforcement, and dimension helpers.
//!
//! # Responsibilities
//!
//! * [`client_width`] / [`client_height`] – total on-screen extent including borders.
//! * [`resize`] – high-level resize that runs size-hint validation first.
//! * [`resize_client`] – low-level X11 configure + state update.
//! * [`apply_size_hints`] – clamp a proposed geometry to ICCCM `WM_NORMAL_HINTS`.
//! * [`update_size_hints`] / [`update_size_hints_win`] – read `WM_NORMAL_HINTS` from X.
//! * [`scale_client`] – resize a client to a percentage of its monitor.

use crate::client::constants::{
    SIZE_HINTS_P_ASPECT, SIZE_HINTS_P_BASE_SIZE, SIZE_HINTS_P_MAX_SIZE, SIZE_HINTS_P_MIN_SIZE,
    SIZE_HINTS_P_RESIZE_INC,
};
use crate::contexts::WmCtx;
use crate::types::{Client, Rect};
use std::cmp::{max, min};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;

// ---------------------------------------------------------------------------
// Dimension helpers
// ---------------------------------------------------------------------------

/// Total width of the client window on screen, including both borders.
#[inline]
pub fn client_width(c: &Client) -> i32 {
    c.geo.w + 2 * c.border_width
}

/// Total height of the client window on screen, including both borders.
#[inline]
pub fn client_height(c: &Client) -> i32 {
    c.geo.h + 2 * c.border_width
}

// ---------------------------------------------------------------------------
// High-level resize (validates size hints)
// ---------------------------------------------------------------------------

/// Resize `win` to the given `rect`, enforcing `WM_NORMAL_HINTS` constraints.
///
/// If the size-hint check determines that nothing changed *and* there is more
/// than one client on screen, the X11 configure call is skipped.  With a
/// single client we always apply the resize so the window fills its space.
pub fn resize(ctx: &mut WmCtx, win: Window, rect: &Rect, interact: bool) {
    // Extract needed data first to avoid borrow conflict
    let (
        base_width,
        base_height,
        min_width,
        min_height,
        max_width,
        max_height,
        inc_width,
        inc_height,
        base_aspect_num,
        base_aspect_denom,
        min_aspect_num,
        min_aspect_denom,
        max_aspect_num,
        max_aspect_denom,
    ) = {
        let client = match ctx.g.clients.get(&win) {
            Some(c) => c,
            None => return,
        };
        (
            client.base_width,
            client.base_height,
            client.min_width,
            client.min_height,
            client.max_width,
            client.max_height,
            client.inc_width,
            client.inc_height,
            client.base_aspect_num,
            client.base_aspect_denom,
            client.min_aspect_num,
            client.min_aspect_denom,
            client.max_aspect_num,
            client.max_aspect_denom,
        )
    };

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
        base_width,
        base_height,
        min_width,
        min_height,
        max_width,
        max_height,
        inc_width,
        inc_height,
        base_aspect_num,
        base_aspect_denom,
        min_aspect_num,
        min_aspect_denom,
        max_aspect_num,
        max_aspect_denom,
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
pub fn resize_client(ctx: &mut WmCtx, win: Window, rect: &Rect) {
    let conn = ctx.x11.conn;

    if let Some(client) = ctx.g.clients.get_mut(&win) {
        // Snapshot old geometry before overwriting.
        client.old_geo.x = client.geo.x;
        client.old_geo.y = client.geo.y;
        client.old_geo.w = client.geo.w;
        client.old_geo.h = client.geo.h;

        client.geo.x = rect.x;
        client.geo.y = rect.y;
        client.geo.w = rect.w;
        client.geo.h = rect.h;

        let border_width = client.border_width;

        let _ = conn.configure_window(
            win,
            &ConfigureWindowAux::new()
                .x(rect.x)
                .y(rect.y)
                .width(rect.w as u32)
                .height(rect.h as u32)
                .border_width(border_width as u32),
        );
    }

    // Send a synthetic ConfigureNotify so the client knows its geometry.
    crate::client::focus::configure(ctx, win);
    let _ = conn.flush();
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
#[allow(clippy::too_many_arguments)]
pub fn apply_size_hints(
    ctx: &mut WmCtx,
    win: Window,
    x: &mut i32,
    y: &mut i32,
    w: &mut i32,
    h: &mut i32,
    interact: bool,
    _base_width: i32,
    _base_height: i32,
    _min_width: i32,
    _min_height: i32,
    _max_width: i32,
    _max_height: i32,
    _inc_width: i32,
    _inc_height: i32,
    _base_aspect_num: i32,
    _base_aspect_denom: i32,
    _min_aspect_num: i32,
    _min_aspect_denom: i32,
    _max_aspect_num: i32,
    _max_aspect_denom: i32,
) -> bool {
    let Some(c) = ctx.g.clients.get_mut(&win) else {
        return false;
    };

    // Snapshot current geometry before any modifications.
    let old_x = c.geo.x;
    let old_y = c.geo.y;
    let old_w = c.geo.w;
    let old_h = c.geo.h;
    let _border_width = c.border_width;
    let client_w = c.geo.w + 2 * c.border_width;
    let client_h = c.geo.h + 2 * c.border_width;
    let mon_id = c.mon_id;
    let hintsvalid = c.hintsvalid;
    let isfloating = c.isfloating;

    // Release the mutable borrow of ctx.g before we might need to call update_size_hints.
    let (cfg, monitors, _tags) = {
        let g = &*ctx.g;
        (g.cfg.clone(), g.monitors.clone(), g.tags.clone())
    };

    *w = max(1, *w);
    *h = max(1, *h);

    // Clamp position so the window doesn't escape the usable area.
    if interact {
        if *x > cfg.screen_width {
            *x = cfg.screen_width - client_w;
        }
        if *y > cfg.screen_height {
            *y = cfg.screen_height - client_h;
        }
        if *x + client_w < 0 {
            *x = 0;
        }
        if *y + client_h < 0 {
            *y = 0;
        }
    } else if let Some(mon_id) = mon_id {
        if let Some(m) = monitors.get(mon_id) {
            if *x >= m.work_rect.x + m.work_rect.w {
                *x = m.work_rect.x + m.work_rect.w - client_w;
            }
            if *y >= m.work_rect.y + m.work_rect.h {
                *y = m.work_rect.y + m.work_rect.h - client_h;
            }
            if *x + client_w <= m.work_rect.x {
                *x = m.work_rect.x;
            }
            if *y + client_h <= m.work_rect.y {
                *y = m.work_rect.y;
            }
        }
    }

    // Enforce a minimum size of one bar-height in each dimension.
    let bh = cfg.bar_height;
    if *h < bh {
        *h = bh;
    }
    if *w < bh {
        *w = bh;
    }

    let resizehints = cfg.resizehints;
    let is_tiling = mon_id
        .and_then(|mid| monitors.get(mid))
        .map(|mon| mon.is_tiling_layout())
        .unwrap_or(true);

    // Need to get mutable client again for the size hints section.
    let Some(c) = ctx.g.clients.get_mut(&win) else {
        return false;
    };

    // Only apply ICCCM size hints when hints are enabled, or the client is
    // floating / not in a tiling layout.
    if resizehints != 0 || isfloating || !is_tiling {
        if hintsvalid == 0 {
            let _ = c; // Release mutable borrow before calling update_size_hints
            update_size_hints(ctx, win);
            let c = ctx.g.clients.get_mut(&win).unwrap();

            let base_is_min =
                c.size_hints.basew == c.size_hints.minw && c.size_hints.baseh == c.size_hints.minh;

            // Step 1: subtract base size before aspect / increment checks.
            if !base_is_min {
                *w -= c.size_hints.basew;
                *h -= c.size_hints.baseh;
            }

            // Step 2: enforce aspect ratio.
            if c.min_aspect > 0.0 && c.max_aspect > 0.0 {
                if c.max_aspect < (*w as f32) / (*h as f32) {
                    *w = (*h as f32 * c.max_aspect + 0.5) as i32;
                } else if c.min_aspect < (*h as f32) / (*w as f32) {
                    *h = (*w as f32 * c.min_aspect + 0.5) as i32;
                }
            }

            // Step 3: when base == min, subtract base *after* the aspect check.
            if base_is_min {
                *w -= c.size_hints.basew;
                *h -= c.size_hints.baseh;
            }

            // Step 4: snap to resize increments.
            if c.size_hints.incw != 0 {
                *w -= *w % c.size_hints.incw;
            }
            if c.size_hints.inch != 0 {
                *h -= *h % c.size_hints.inch;
            }

            // Step 5: re-add base and clamp to [min, max].
            *w = max(*w + c.size_hints.basew, c.size_hints.minw);
            *h = max(*h + c.size_hints.baseh, c.size_hints.minh);

            if c.size_hints.maxw != 0 {
                *w = min(*w, c.size_hints.maxw);
            }
            if c.size_hints.maxh != 0 {
                *h = min(*h, c.size_hints.maxh);
            }
        } else {
            // hintsvalid != 0, already have valid hints
            let base_is_min =
                c.size_hints.basew == c.size_hints.minw && c.size_hints.baseh == c.size_hints.minh;

            // Step 1: subtract base size before aspect / increment checks.
            if !base_is_min {
                *w -= c.size_hints.basew;
                *h -= c.size_hints.baseh;
            }

            // Step 2: enforce aspect ratio.
            if c.min_aspect > 0.0 && c.max_aspect > 0.0 {
                if c.max_aspect < (*w as f32) / (*h as f32) {
                    *w = (*h as f32 * c.max_aspect + 0.5) as i32;
                } else if c.min_aspect < (*h as f32) / (*w as f32) {
                    *h = (*w as f32 * c.min_aspect + 0.5) as i32;
                }
            }

            // Step 3: when base == min, subtract base *after* the aspect check.
            if base_is_min {
                *w -= c.size_hints.basew;
                *h -= c.size_hints.baseh;
            }

            // Step 4: snap to resize increments.
            if c.size_hints.incw != 0 {
                *w -= *w % c.size_hints.incw;
            }
            if c.size_hints.inch != 0 {
                *h -= *h % c.size_hints.inch;
            }

            // Step 5: re-add base and clamp to [min, max].
            *w = max(*w + c.size_hints.basew, c.size_hints.minw);
            *h = max(*h + c.size_hints.baseh, c.size_hints.minh);

            if c.size_hints.maxw != 0 {
                *w = min(*w, c.size_hints.maxw);
            }
            if c.size_hints.maxh != 0 {
                *h = min(*h, c.size_hints.maxh);
            }
        }
    }

    *x != old_x || *y != old_y || *w != old_w || *h != old_h
}

// ---------------------------------------------------------------------------
// WM_NORMAL_HINTS parsing
// ---------------------------------------------------------------------------

/// Read `WM_NORMAL_HINTS` from the X server and populate the client's size hints,
/// `min_aspect`, `max_aspect`, and `isfixed`.
///
/// The raw property is a packed C struct; we read individual 4-byte integers
/// at well-known byte offsets defined by the ICCCM / Xlib `XSizeHints` layout.
pub fn update_size_hints(ctx: &mut WmCtx, win: Window) {
    let conn = ctx.x11.conn;

    let Some(c) = ctx.g.clients.get_mut(&win) else {
        return;
    };
    let cwin = c.win;

    let Ok(cookie) = conn.get_property(
        false,
        cwin,
        AtomEnum::WM_NORMAL_HINTS,
        AtomEnum::WM_SIZE_HINTS,
        0,
        24,
    ) else {
        return;
    };

    let Ok(reply) = cookie.reply() else { return };

    let data: Vec<u32> = reply.value32().map(|v| v.collect()).unwrap_or_default();

    // Helper: read a u32 at index `idx`, or 0 if out of range.
    let read_i32 = |idx: usize| -> i32 {
        if data.len() > idx {
            data[idx] as i32
        } else {
            0
        }
    };

    let flags = if !data.is_empty() { data[0] } else { 0 };

    // Re-acquire mutable reference.
    let c = match ctx.g.clients.get_mut(&win) {
        Some(c) => c,
        None => return,
    };

    // --- base size (index 15) / min size (index 5) ---
    if flags & SIZE_HINTS_P_BASE_SIZE != 0 && data.len() > 16 {
        c.size_hints.basew = read_i32(15);
        c.size_hints.baseh = read_i32(16);
    } else if flags & SIZE_HINTS_P_MIN_SIZE != 0 && data.len() > 6 {
        // Fall back to min size when base size is absent.
        c.size_hints.basew = read_i32(5);
        c.size_hints.baseh = read_i32(6);
    } else {
        c.size_hints.basew = 0;
        c.size_hints.baseh = 0;
    }

    // --- resize increments (index 9) ---
    if flags & SIZE_HINTS_P_RESIZE_INC != 0 && data.len() > 10 {
        c.size_hints.incw = read_i32(9);
        c.size_hints.inch = read_i32(10);
    } else {
        c.size_hints.incw = 0;
        c.size_hints.inch = 0;
    }

    // --- max size (index 7) ---
    if flags & SIZE_HINTS_P_MAX_SIZE != 0 && data.len() > 8 {
        c.size_hints.maxw = read_i32(7);
        c.size_hints.maxh = read_i32(8);
    } else {
        c.size_hints.maxw = 0;
        c.size_hints.maxh = 0;
    }

    // --- min size (index 5) ---
    if flags & SIZE_HINTS_P_MIN_SIZE != 0 && data.len() > 6 {
        c.size_hints.minw = read_i32(5);
        c.size_hints.minh = read_i32(6);
    } else if flags & SIZE_HINTS_P_BASE_SIZE != 0 && data.len() > 16 {
        // Fall back to base size when min size is absent.
        c.size_hints.minw = c.size_hints.basew;
        c.size_hints.minh = c.size_hints.baseh;
    } else {
        c.size_hints.minw = 0;
        c.size_hints.minh = 0;
    }

    // --- aspect ratio (indices 11 / 12 / 13 / 14) ---
    if flags & SIZE_HINTS_P_ASPECT != 0 && data.len() > 14 {
        let min_aspect_num = read_i32(11);
        let min_aspect_denom = read_i32(12);
        let max_aspect_num = read_i32(13);
        let max_aspect_denom = read_i32(14);

        c.min_aspect = if min_aspect_denom != 0 {
            min_aspect_num as f32 / min_aspect_denom as f32
        } else {
            0.0
        };
        c.max_aspect = if max_aspect_denom != 0 {
            max_aspect_num as f32 / max_aspect_denom as f32
        } else {
            0.0
        };
    } else {
        c.min_aspect = 0.0;
        c.max_aspect = 0.0;
    }

    // A client is "fixed size" when its max and min dimensions are identical
    // and non-zero – it cannot be resized at all.
    c.isfixed = c.size_hints.maxw != 0
        && c.size_hints.maxh != 0
        && c.size_hints.maxw == c.size_hints.minw
        && c.size_hints.maxh == c.size_hints.minh;

    c.hintsvalid = 1;
}

/// Convenience wrapper: look up `win` in the global client map and call
/// [`update_size_hints`] on the found [`Client`].
pub fn update_size_hints_win(ctx: &mut WmCtx, win: Window) {
    update_size_hints(ctx, win);
}

// ---------------------------------------------------------------------------
// Scale helper
// ---------------------------------------------------------------------------

/// Resize `win` to `scale` percent of its monitor dimensions, centred on screen.
///
/// `scale` is an integer percentage (e.g. `75` means 75 %).
pub fn scale_client(ctx: &mut WmCtx, win: Window, scale: i32) {
    let (mon_id, old_geo, border_width) = {
        let c = match ctx.g.clients.get(&win) {
            Some(c) => c,
            None => return,
        };
        (c.mon_id, c.geo, c.border_width)
    };

    // Determine the reference rectangle (monitor bounds, or fall back to the
    // client's own geometry when no monitor is assigned).
    let mon_rect = mon_id
        .and_then(|mid| ctx.g.monitor(mid).map(|m| m.monitor_rect))
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
