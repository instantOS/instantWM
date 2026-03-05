//! Interactive mouse-resize operations.
//!
//! Three distinct resize modes are provided:
//!
//! | Function                  | Description                                                  |
//! |---------------------------|--------------------------------------------------------------|
//! | [`resize_mouse`]          | Drag the bottom-right corner to resize                      |
//! | [`resize_aspect_mouse`]   | Same, but clamps to the window's declared aspect-ratio hints |
//! | [`force_resize_mouse`]    | Alias for `resize_mouse` (bypasses fullscreen guard)        |
//!
//! All three share the same grab/event-loop/ungrab skeleton; they differ only
//! in how they compute the new width and height from the pointer position.
//!
//! # Resize-direction constants
//!
//! The `RESIZE_DIR_*` constants in [`super::constants`] identify which corner
//! or edge is being dragged.  Currently only `RESIZE_DIR_BOTTOM_RIGHT` is used
//! by the interactive loops; the others are reserved for future directional
//! resize support.

use crate::client::resize;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::floating::toggle_floating;
use crate::types::*;
use x11rb::protocol::xproto::*;

use super::constants::{REFRESH_RATE_HI, REFRESH_RATE_LO};
use super::grab::{grab_pointer, ungrab, wait_event};
use super::monitor::handle_client_monitor_switch;
use crate::types::ResizeDirection;

// ── Shared helpers ────────────────────────────────────────────────────────────

pub fn resize_mouse_from_cursor(ctx: &mut WmCtxX11, btn: MouseButton) {
    let Some(win) = ctx.core.selected_client() else {
        return;
    };
    let is_blocked = ctx
        .core
        .g
        .clients
        .get(&win)
        .map(|c| c.is_true_fullscreen())
        .unwrap_or(false);
    if is_blocked {
        return;
    };

    let dir = {
        let Some(c) = ctx.core.g.clients.get(&win) else {
            return;
        };

        let conn = ctx.x11.conn;
        let x11_win: Window = win.into();
        let Ok(cookie) = conn.query_pointer(x11_win) else {
            return;
        };
        let Ok(reply) = cookie.reply() else { return };

        let hit_x = reply.win_x as i32;
        let hit_y = reply.win_y as i32;

        get_resize_direction(c.geo.w, c.geo.h, hit_x, hit_y)
    };

    resize_mouse_directional(ctx, Some(dir), btn);
}

/// Decide the motion-event throttle based on `globals.doubledraw`.
fn refresh_rate(ctx: &WmCtx) -> u32 {
    if ctx.g.doubledraw {
        REFRESH_RATE_HI
    } else {
        REFRESH_RATE_LO
    }
}

// ── resize_mouse ─────────────────────────────────────────────────────────────

/// Interactive bottom-right-corner resize.
///
/// Grabs the pointer, then for every `MotionNotify` event computes a new
/// `(w, h)` from the distance between the pointer and the window's top-left
/// corner.  If the window is tiled and the delta exceeds the snap threshold,
/// it is promoted to floating first.
///
/// The loop ends on `ButtonRelease`.  After the grab is released,
/// [`handle_client_monitor_switch`] checks whether the window crossed a monitor
/// boundary during the resize.
pub fn resize_mouse(ctx: &mut WmCtxX11, btn: MouseButton) {
    let Some(win) = ctx.core.selected_client() else {
        return;
    };
    let is_blocked = ctx
        .core
        .g
        .clients
        .get(&win)
        .map(|c| c.is_true_fullscreen())
        .unwrap_or(false);
    if is_blocked {
        return;
    };

    crate::layouts::restack(ctx, ctx.core.g.selected_monitor_id());

    if !grab_pointer(ctx, 1) {
        return;
    }

    let (orig_left, orig_top) = {
        match ctx.core.g.clients.get(&win) {
            Some(c) => (c.geo.x, c.geo.y),
            None => {
                ungrab(ctx);
                return;
            }
        }
    };

    let rate = refresh_rate(ctx);
    let mut last_time: u32 = 0;

    loop {
        let Some(event) = wait_event(ctx) else {
            break;
        };

        match &event {
            x11rb::protocol::Event::ButtonRelease(br) => {
                if br.detail == btn.as_u8() {
                    break;
                }
            }

            x11rb::protocol::Event::MotionNotify(m) => {
                if m.time - last_time <= crate::constants::animation::MOUSE_EVENT_RATE {
                    continue;
                }
                last_time = m.time;

                let nw = (m.event_x as i32 - orig_left + 1).max(1);
                let nh = (m.event_y as i32 - orig_top + 1).max(1);

                let snap = ctx.g.cfg.snap;

                if let Some(client) = ctx.g.clients.get(&win) {
                    let has_tiling = ctx.g.selected_monitor().is_tiling_layout();

                    if !client.isfloating
                        && has_tiling
                        && ((nw - client.geo.w).abs() > snap || (nh - client.geo.h).abs() > snap)
                    {
                        toggle_floating(ctx);
                    } else if !has_tiling || client.isfloating {
                        resize(
                            ctx,
                            win,
                            &Rect {
                                x: client.geo.x,
                                y: client.geo.y,
                                w: nw,
                                h: nh,
                            },
                            true,
                        );
                    }
                }
            }

            _ => {}
        }
    }

    ungrab(ctx);
    handle_client_monitor_switch(ctx, win);
}

/// Directional resize: supports all 8 directions (corners and edges).
///
/// When `direction` is `None`, behaves like [`resize_mouse`] (bottom-right corner).
/// Otherwise, resizes from the specified edge or corner.
pub fn resize_mouse_directional(
    ctx: &mut WmCtxX11,
    direction: Option<ResizeDirection>,
    btn: MouseButton,
) {
    let Some(win) = ctx.core.selected_client() else {
        return;
    };
    let is_blocked = ctx
        .core
        .g
        .clients
        .get(&win)
        .map(|c| c.is_true_fullscreen())
        .unwrap_or(false);
    if is_blocked {
        return;
    };

    crate::layouts::restack(ctx, ctx.core.g.selected_monitor_id());

    if !grab_pointer(ctx, 1) {
        return;
    }

    let (orig_left, orig_top, orig_right, orig_bottom, border_width) = {
        match ctx.core.g.clients.get(&win) {
            Some(c) => (
                c.geo.x,
                c.geo.y,
                c.geo.x + c.geo.w,
                c.geo.y + c.geo.h,
                c.border_width,
            ),
            None => {
                ungrab(ctx);
                return;
            }
        }
    };

    let dir = direction.unwrap_or(ResizeDirection::BottomRight);
    let (affects_left, affects_right, affects_top, affects_bottom) = dir.affected_edges();

    let rate = refresh_rate(ctx);
    let mut last_time: u32 = 0;

    loop {
        let Some(event) = wait_event(ctx) else {
            break;
        };

        match &event {
            x11rb::protocol::Event::ButtonRelease(br) => {
                if br.detail == btn.as_u8() {
                    break;
                }
            }

            x11rb::protocol::Event::MotionNotify(m) => {
                if m.time - last_time <= crate::constants::animation::MOUSE_EVENT_RATE {
                    continue;
                }
                last_time = m.time;

                let pointer_x = m.event_x as i32;
                let pointer_y = m.event_y as i32;

                let (new_x, new_w) = if affects_left {
                    let nx = pointer_x;
                    let nw = (orig_right - pointer_x).max(1);
                    (nx, nw)
                } else if affects_right {
                    let nw = (pointer_x - orig_left - 2 * border_width + 1).max(1);
                    (orig_left, nw)
                } else {
                    (orig_left, orig_right - orig_left)
                };

                let (new_y, new_h) = if affects_top {
                    let ny = pointer_y;
                    let nh = (orig_bottom - pointer_y).max(1);
                    (ny, nh)
                } else if affects_bottom {
                    let nh = (pointer_y - orig_top - 2 * border_width + 1).max(1);
                    (orig_top, nh)
                } else {
                    (orig_top, orig_bottom - orig_top)
                };

                let snap = ctx.g.cfg.snap;

                let should_toggle = if let Some(client) = ctx.g.clients.get(&win) {
                    let has_tiling = ctx.g.selected_monitor().is_tiling_layout();

                    !client.isfloating
                        && has_tiling
                        && ((new_w - client.geo.w).abs() > snap
                            || (new_h - client.geo.h).abs() > snap)
                } else {
                    false
                };

                if should_toggle {
                    toggle_floating(ctx);
                } else {
                    let is_floating = ctx
                        .g
                        .clients
                        .get(&win)
                        .map(|c| c.isfloating)
                        .unwrap_or(false);
                    let has_tiling = ctx.g.selected_monitor().is_tiling_layout();

                    if !has_tiling || is_floating {
                        resize(
                            ctx,
                            win,
                            &Rect {
                                x: new_x,
                                y: new_y,
                                w: new_w,
                                h: new_h,
                            },
                            true,
                        );
                    }
                }
            }

            _ => {}
        }
    }

    ungrab(ctx);
    handle_client_monitor_switch(ctx, win);
}

/// Alias for [`resize_mouse`].
///
/// Exists to match the C API where `force_resize_mouse` was a separate symbol
/// that bypassed an additional fullscreen guard.  The Rust version already
/// handles this cleanly in [`resize_mouse`].
#[inline]
pub fn force_resize_mouse(ctx: &mut WmCtx, btn: MouseButton) {
    resize_mouse(ctx, btn);
}

// ── resize_aspect_mouse ───────────────────────────────────────────────────────

/// Interactive resize that respects the window's declared aspect-ratio hints.
///
/// Reads `client.min_aspect`, `client.max_aspect`, and `client.size_hints` to clamp the
/// new dimensions so the window's aspect ratio stays within the range it
/// advertised via `WM_NORMAL_HINTS`.
///
/// Unlike [`resize_mouse`] this function does **not** toggle floating; it is
/// intended for use on windows that are already floating (e.g. video players
/// with a fixed aspect ratio).
pub fn resize_aspect_mouse(ctx: &mut WmCtxX11, win: WindowId, btn: MouseButton) {
    let is_fullscreen = ctx
        .core
        .g
        .clients
        .get(&win)
        .map(|c| c.is_fullscreen)
        .unwrap_or(false);
    if is_fullscreen {
        return;
    };

    crate::layouts::restack(ctx, ctx.core.g.selected_monitor_id());

    if !grab_pointer(ctx, 1) {
        return;
    }

    let (orig_left, orig_top) = {
        match ctx.core.g.clients.get(&win) {
            Some(c) => (c.geo.x, c.geo.y),
            None => {
                ungrab(ctx);
                return;
            }
        }
    };

    let rate = refresh_rate(ctx);
    let mut last_time: u32 = 0;

    loop {
        let Some(event) = wait_event(ctx) else {
            break;
        };

        match &event {
            x11rb::protocol::Event::ButtonRelease(br) => {
                if br.detail == btn.as_u8() {
                    break;
                }
            }

            x11rb::protocol::Event::MotionNotify(m) => {
                if m.time - last_time <= crate::constants::animation::MOUSE_EVENT_RATE {
                    continue;
                }
                last_time = m.time;

                let raw_nw = (m.event_x as i32 - orig_left + 1).max(1);
                let raw_nh = (m.event_y as i32 - orig_top + 1).max(1);

                if let Some(client) = ctx.g.clients.get(&win) {
                    let sh = &client.size_hints;
                    let (min_aspect, max_aspect) = (client.min_aspect, client.max_aspect);

                    let mut nw = raw_nw;
                    let mut nh = raw_nh;

                    // Clamp to declared min/max dimensions.
                    if sh.minw > 0 {
                        nw = nw.max(sh.minw);
                    }
                    if sh.minh > 0 {
                        nh = nh.max(sh.minh);
                    }
                    if sh.maxw > 0 {
                        nw = nw.min(sh.maxw);
                    }
                    if sh.maxh > 0 {
                        nh = nh.min(sh.maxh);
                    }

                    // Clamp to declared aspect-ratio range.
                    if min_aspect > 0.0 && max_aspect > 0.0 {
                        if max_aspect < nw as f32 / nh as f32 {
                            nw = (nh as f32 * max_aspect) as i32;
                        } else if min_aspect < nh as f32 / nw as f32 {
                            nh = (nw as f32 * min_aspect) as i32;
                        }
                    }

                    resize(
                        ctx,
                        win,
                        &Rect {
                            x: client.geo.x,
                            y: client.geo.y,
                            w: nw,
                            h: nh,
                        },
                        true,
                    );
                }
            }

            _ => {}
        }
    }

    ungrab(ctx);
    handle_client_monitor_switch(ctx, win);
}

// `hover_resize_mouse` and `is_in_resize_border` live in `super::hover`.
