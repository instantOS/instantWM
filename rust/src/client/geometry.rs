//! Client geometry: resizing, size-hint enforcement, and dimension helpers.
//!
//! # Responsibilities
//!
//! * [`client_width`] / [`client_height`] – total on-screen extent including borders.
//! * [`resize`] – high-level resize that runs size-hint validation first.
//! * [`resize_client`] / [`resize_client_rect`] – low-level X11 configure + state update.
//! * [`apply_size_hints`] – clamp a proposed geometry to ICCCM `WM_NORMAL_HINTS`.
//! * [`update_size_hints`] / [`update_size_hints_win`] – read `WM_NORMAL_HINTS` from X.
//! * [`scale_client`] – resize a client to a percentage of its monitor.

use crate::client::constants::{
    SIZE_HINTS_P_ASPECT, SIZE_HINTS_P_BASE_SIZE, SIZE_HINTS_P_MAX_SIZE, SIZE_HINTS_P_MIN_SIZE,
    SIZE_HINTS_P_RESIZE_INC,
};
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::types::{Client, Rect};
use crate::util::{max, min};
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
pub fn resize(win: Window, rect: &Rect, interact: bool) {
    let globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        let mut new_x = rect.x;
        let mut new_y = rect.y;
        let mut new_width = rect.w;
        let mut new_height = rect.h;
        let changed = apply_size_hints(
            client,
            &mut new_x,
            &mut new_y,
            &mut new_width,
            &mut new_height,
            interact,
        );
        let client_count = globals.clients.len();
        if changed || client_count == 1 {
            resize_client_rect(
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
pub fn resize_client(win: Window, x: i32, y: i32, w: i32, h: i32) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            // Snapshot old geometry before overwriting.
            client.old_geo.x = client.geo.x;
            client.old_geo.y = client.geo.y;
            client.old_geo.w = client.geo.w;
            client.old_geo.h = client.geo.h;

            client.geo.x = x;
            client.geo.y = y;
            client.geo.w = w;
            client.geo.h = h;

            let border_width = client.border_width;

            let _ = conn.configure_window(
                win,
                &ConfigureWindowAux::new()
                    .x(x)
                    .y(y)
                    .width(w as u32)
                    .height(h as u32)
                    .border_width(border_width as u32),
            );
        }

        // Send a synthetic ConfigureNotify so the client knows its geometry.
        crate::client::focus::configure(win);
        let _ = conn.flush();
    }
}

/// Resize a client using a [`Rect`] – thin wrapper around [`resize_client`].
#[inline]
pub fn resize_client_rect(win: Window, rect: &Rect) {
    resize_client(win, rect.x, rect.y, rect.w, rect.h);
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
    c: &mut Client,
    x: &mut i32,
    y: &mut i32,
    w: &mut i32,
    h: &mut i32,
    interact: bool,
) -> bool {
    let globals = get_globals();

    *w = max(1, *w);
    *h = max(1, *h);

    // Clamp position so the window doesn't escape the usable area.
    if interact {
        if *x > globals.sw {
            *x = globals.sw - client_width(c);
        }
        if *y > globals.sh {
            *y = globals.sh - client_height(c);
        }
        if *x + *w + 2 * c.border_width < 0 {
            *x = 0;
        }
        if *y + *h + 2 * c.border_width < 0 {
            *y = 0;
        }
    } else if let Some(mon_id) = c.mon_id {
        if let Some(m) = globals.monitors.get(mon_id) {
            if *x >= m.work_rect.x + m.work_rect.w {
                *x = m.work_rect.x + m.work_rect.w - client_width(c);
            }
            if *y >= m.work_rect.y + m.work_rect.h {
                *y = m.work_rect.y + m.work_rect.h - client_height(c);
            }
            if *x + *w + 2 * c.border_width <= m.work_rect.x {
                *x = m.work_rect.x;
            }
            if *y + *h + 2 * c.border_width <= m.work_rect.y {
                *y = m.work_rect.y;
            }
        }
    }

    // Enforce a minimum size of one bar-height in each dimension.
    let bh = globals.bh;
    if *h < bh {
        *h = bh;
    }
    if *w < bh {
        *w = bh;
    }

    let resizehints = globals.resizehints;
    let is_tiling = c
        .mon_id
        .and_then(|mid| globals.monitors.get(mid))
        .map(|mon| crate::monitor::is_current_layout_tiling(mon, &globals.tags))
        .unwrap_or(true);

    // Only apply ICCCM size hints when hints are enabled, or the client is
    // floating / not in a tiling layout.
    if resizehints != 0 || c.isfloating || !is_tiling {
        if c.hintsvalid == 0 {
            update_size_hints(c);
        }

        let base_is_min =
            c.size_hints.basew == c.size_hints.minw && c.size_hints.baseh == c.size_hints.minh;

        // Step 1: subtract base size before aspect / increment checks.
        if !base_is_min {
            *w -= c.size_hints.basew;
            *h -= c.size_hints.baseh;
        }

        // Step 2: enforce aspect ratio.
        if c.mina > 0.0 && c.maxa > 0.0 {
            if c.maxa < (*w as f32) / (*h as f32) {
                *w = (*h as f32 * c.maxa + 0.5) as i32;
            } else if c.mina < (*h as f32) / (*w as f32) {
                *h = (*w as f32 * c.mina + 0.5) as i32;
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

    *x != c.geo.x || *y != c.geo.y || *w != c.geo.w || *h != c.geo.h
}

// ---------------------------------------------------------------------------
// WM_NORMAL_HINTS parsing
// ---------------------------------------------------------------------------

/// Read `WM_NORMAL_HINTS` from the X server and populate `c.size_hints`,
/// `c.mina`, `c.maxa`, and `c.isfixed`.
///
/// The raw property is a packed C struct; we read individual 4-byte integers
/// at well-known byte offsets defined by the ICCCM / Xlib `XSizeHints` layout.
pub fn update_size_hints(c: &mut Client) {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    let Ok(cookie) = conn.get_property(
        false,
        c.win,
        AtomEnum::WM_NORMAL_HINTS,
        AtomEnum::WM_SIZE_HINTS,
        0,
        24,
    ) else {
        return;
    };

    let Ok(reply) = cookie.reply() else { return };

    let data: Vec<u8> = reply.value8().map(|v| v.collect()).unwrap_or_default();

    // Helper: read a little-endian i32 at byte offset `off`, or 0 if out of range.
    let read_i32 = |off: usize| -> i32 {
        if data.len() >= off + 4 {
            i32::from_ne_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
        } else {
            0
        }
    };

    let flags = if data.len() >= 4 {
        u32::from_ne_bytes([data[0], data[1], data[2], data[3]])
    } else {
        0
    };

    // --- base size (byte offset 8) / min size (byte offset 16) ---
    if flags & SIZE_HINTS_P_BASE_SIZE != 0 && data.len() >= 16 {
        c.size_hints.basew = read_i32(8);
        c.size_hints.baseh = read_i32(12);
    } else if flags & SIZE_HINTS_P_MIN_SIZE != 0 && data.len() >= 24 {
        // Fall back to min size when base size is absent.
        c.size_hints.basew = read_i32(16);
        c.size_hints.baseh = read_i32(20);
    } else {
        c.size_hints.basew = 0;
        c.size_hints.baseh = 0;
    }

    // --- resize increments (byte offset 24) ---
    if flags & SIZE_HINTS_P_RESIZE_INC != 0 && data.len() >= 32 {
        c.size_hints.incw = read_i32(24);
        c.size_hints.inch = read_i32(28);
    } else {
        c.size_hints.incw = 0;
        c.size_hints.inch = 0;
    }

    // --- max size (byte offset 32) ---
    if flags & SIZE_HINTS_P_MAX_SIZE != 0 && data.len() >= 40 {
        c.size_hints.maxw = read_i32(32);
        c.size_hints.maxh = read_i32(36);
    } else {
        c.size_hints.maxw = 0;
        c.size_hints.maxh = 0;
    }

    // --- min size (byte offset 16) ---
    if flags & SIZE_HINTS_P_MIN_SIZE != 0 && data.len() >= 24 {
        c.size_hints.minw = read_i32(16);
        c.size_hints.minh = read_i32(20);
    } else if flags & SIZE_HINTS_P_BASE_SIZE != 0 && data.len() >= 16 {
        // Fall back to base size when min size is absent.
        c.size_hints.minw = c.size_hints.basew;
        c.size_hints.minh = c.size_hints.baseh;
    } else {
        c.size_hints.minw = 0;
        c.size_hints.minh = 0;
    }

    // --- aspect ratio (byte offsets 48 / 52 / 56 / 60) ---
    if flags & SIZE_HINTS_P_ASPECT != 0 && data.len() >= 64 {
        let min_aspect_y = read_i32(48);
        let min_aspect_x = read_i32(52);
        let max_aspect_x = read_i32(56);
        let max_aspect_y = read_i32(60);

        c.mina = if min_aspect_x != 0 {
            min_aspect_y as f32 / min_aspect_x as f32
        } else {
            0.0
        };
        c.maxa = if max_aspect_y != 0 {
            max_aspect_x as f32 / max_aspect_y as f32
        } else {
            0.0
        };
    } else {
        c.mina = 0.0;
        c.maxa = 0.0;
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
pub fn update_size_hints_win(win: Window) {
    let globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        update_size_hints(client);
    }
}

// ---------------------------------------------------------------------------
// Scale helper
// ---------------------------------------------------------------------------

/// Resize `win` to `scale` percent of its monitor dimensions, centred on screen.
///
/// `scale` is an integer percentage (e.g. `75` means 75 %).
pub fn scale_client(win: Window, scale: i32) {
    let globals = get_globals_mut();
    let Some(client) = globals.clients.get_mut(&win) else {
        return;
    };

    let mon_id = client.mon_id;
    let old_geo = client.geo;
    let border_width = client.border_width;

    // Determine the reference rectangle (monitor bounds, or fall back to the
    // client's own geometry when no monitor is assigned).
    let mon_rect = mon_id
        .and_then(|mid| get_globals().monitors.get(mid).map(|m| m.monitor_rect))
        .unwrap_or(old_geo);

    let new_w = old_geo.w * scale / 100;
    let new_h = old_geo.h * scale / 100;
    let new_x = mon_rect.x + (mon_rect.w - new_w) / 2 - border_width;
    let new_y = mon_rect.y + (mon_rect.h - new_h) / 2 - border_width;

    resize(
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
