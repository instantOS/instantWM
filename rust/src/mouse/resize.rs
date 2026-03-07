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
//! On Wayland, `resize_mouse_from_cursor` and `resize_aspect_mouse` bypass the
//! title-drag state machine and instead directly activate a
//! `HoverResizeDragState`.  This reuses the same directional-resize event loop
//! that hover-border drags use, giving correct per-quadrant behaviour without
//! any cursor warp or anchor chaos.

use crate::client::resize;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::floating::toggle_floating;
use crate::types::*;
use x11rb::protocol::xproto::*;

use super::constants::{REFRESH_RATE_HI, REFRESH_RATE_LO};
use super::cursor::{set_cursor_move_wayland, set_cursor_resize_wayland};
use super::grab::{grab_pointer, ungrab, wait_event};
use super::monitor::handle_client_monitor_switch;
use crate::types::input::get_resize_direction;
use crate::types::ResizeDirection;

fn with_wm_ctx_x11<T>(ctx_x11: &mut WmCtxX11<'_>, f: impl FnOnce(&mut WmCtx<'_>) -> T) -> T {
    let mut ctx = WmCtx::X11(ctx_x11.reborrow());
    f(&mut ctx)
}

// ── Shared helpers ────────────────────────────────────────────────────────────

pub fn resize_mouse_from_cursor(ctx: &mut WmCtx, btn: MouseButton) {
    let Some(win) = ctx.selected_client() else {
        return;
    };
    let is_blocked = ctx
        .g()
        .clients
        .get(&win)
        .map(|c| c.is_true_fullscreen())
        .unwrap_or(false);
    if is_blocked {
        return;
    };

    match ctx {
        WmCtx::X11(ctx_x11) => {
            let dir = {
                let Some(c) = ctx_x11.core.g.clients.get(&win) else {
                    return;
                };

                let conn = ctx_x11.x11.conn;
                let x11_win: Window = win.into();
                let Ok(cookie) = conn.query_pointer(x11_win) else {
                    return;
                };
                let Ok(reply) = cookie.reply() else { return };

                let hit_x = reply.win_x as i32;
                let hit_y = reply.win_y as i32;

                get_resize_direction(c.geo.w, c.geo.h, hit_x, hit_y)
            };

            resize_mouse_directional(ctx_x11, Some(dir), btn);
        }
        WmCtx::Wayland(wl) => {
            // Get the current pointer position and compute the resize direction
            // from which quadrant of the window it falls in.
            let Some((ptr_x, ptr_y)) = wl.wayland.backend.pointer_location() else {
                return;
            };
            let Some((geo, is_floating, border_width)) = wl
                .core
                .g
                .clients
                .get(&win)
                .map(|c| (c.geo, c.isfloating, c.border_width))
            else {
                return;
            };

            // Promote tiled windows to floating before starting the resize.
            let has_tiling = wl.core.g.selected_monitor().is_tiling_layout();
            if !is_floating && has_tiling {
                let mut wmctx = WmCtx::Wayland(wl.reborrow());
                crate::floating::toggle_floating(&mut wmctx);
                let selmon_id = wmctx.g().selected_monitor_id();
                crate::layouts::arrange(&mut wmctx, Some(selmon_id));
                // Re-read geometry after the layout change.
                let Some(new_geo) = wmctx.g().clients.get(&win).map(|c| c.geo) else {
                    return;
                };
                let hit_x = ptr_x - new_geo.x;
                let hit_y = ptr_y - new_geo.y;
                let dir = get_resize_direction(new_geo.w, new_geo.h, hit_x, hit_y);
                if let WmCtx::Wayland(wl2) = wmctx {
                    begin_wayland_super_resize(wl2, win, btn, dir, new_geo, ptr_x, ptr_y);
                }
                return;
            }

            let hit_x = ptr_x - geo.x;
            let hit_y = ptr_y - geo.y;
            let dir = get_resize_direction(geo.w, geo.h, hit_x, hit_y);
            begin_wayland_super_resize(wl, win, btn, dir, geo, ptr_x, ptr_y);
            let _ = border_width; // used via HoverResizeDragState
        }
    }
}

/// Activate a `HoverResizeDragState` for a Super+RMB resize initiated anywhere
/// on a Wayland window (not just the hover-border zone).  This reuses the same
/// directional-resize event loop as hover-border resizes, giving correct
/// per-quadrant behaviour with no cursor warp.
fn begin_wayland_super_resize(
    wl: &mut crate::contexts::WmCtxWayland<'_>,
    win: WindowId,
    btn: MouseButton,
    dir: ResizeDirection,
    geo: Rect,
    ptr_x: i32,
    ptr_y: i32,
) {
    wl.core.g.drag.hover_resize = crate::globals::HoverResizeDragState {
        active: true,
        win,
        button: btn,
        direction: dir,
        move_mode: false,
        start_x: ptr_x,
        start_y: ptr_y,
        win_start_geo: geo,
        last_root_x: ptr_x,
        last_root_y: ptr_y,
    };
    wl.core.g.altcursor = AltCursor::Resize;
    wl.core.g.drag.resize_direction = Some(dir);
    set_cursor_resize_wayland(wl, Some(dir));
    let _ = crate::focus::focus_wayland(&mut wl.core, &wl.wayland, Some(win));
    let mut wmctx = WmCtx::Wayland(wl.reborrow());
    wmctx.raise_interactive(win);
}

/// Decide the motion-event throttle based on `globals.doubledraw`.
fn refresh_rate(ctx: &WmCtxX11) -> u32 {
    if ctx.core.g.doubledraw {
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

    with_wm_ctx_x11(ctx, |ctx| ctx.raise_interactive(win));

    with_wm_ctx_x11(ctx, |ctx| {
        crate::layouts::restack(ctx, ctx.g().selected_monitor_id())
    });

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

    let _rate = refresh_rate(ctx);
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

                let snap = ctx.core.g.cfg.snap;

                let has_tiling = ctx.core.g.selected_monitor().is_tiling_layout();
                if let Some((client_geo, is_floating)) =
                    ctx.core.g.clients.get(&win).map(|c| (c.geo, c.isfloating))
                {
                    if !is_floating
                        && has_tiling
                        && ((nw - client_geo.w).abs() > snap || (nh - client_geo.h).abs() > snap)
                    {
                        with_wm_ctx_x11(ctx, |ctx| toggle_floating(ctx));
                    } else if !has_tiling || is_floating {
                        with_wm_ctx_x11(ctx, |ctx| {
                            resize(
                                ctx,
                                win,
                                &Rect {
                                    x: client_geo.x,
                                    y: client_geo.y,
                                    w: nw,
                                    h: nh,
                                },
                                true,
                            );
                        });
                    }
                }
            }

            _ => {}
        }
    }

    ungrab(ctx);
    with_wm_ctx_x11(ctx, |ctx| handle_client_monitor_switch(ctx, win));
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

    with_wm_ctx_x11(ctx, |ctx| ctx.raise_interactive(win));

    with_wm_ctx_x11(ctx, |ctx| {
        crate::layouts::restack(ctx, ctx.g().selected_monitor_id())
    });

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

    let _rate = refresh_rate(ctx);
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

                let snap = ctx.core.g.cfg.snap;

                let should_toggle = if let Some(client) = ctx.core.g.clients.get(&win) {
                    let has_tiling = ctx.core.g.selected_monitor().is_tiling_layout();

                    !client.isfloating
                        && has_tiling
                        && ((new_w - client.geo.w).abs() > snap
                            || (new_h - client.geo.h).abs() > snap)
                } else {
                    false
                };

                if should_toggle {
                    with_wm_ctx_x11(ctx, |ctx| toggle_floating(ctx));
                } else {
                    let is_floating = ctx
                        .core
                        .g
                        .clients
                        .get(&win)
                        .map(|c| c.isfloating)
                        .unwrap_or(false);
                    let has_tiling = ctx.core.g.selected_monitor().is_tiling_layout();

                    if !has_tiling || is_floating {
                        with_wm_ctx_x11(ctx, |ctx| {
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
                        });
                    }
                }
            }

            _ => {}
        }
    }

    ungrab(ctx);
    with_wm_ctx_x11(ctx, |ctx| handle_client_monitor_switch(ctx, win));
}

/// Alias for [`resize_mouse`].
///
/// Exists to match the C API where `force_resize_mouse` was a separate symbol
/// that bypassed an additional fullscreen guard.  The Rust version already
/// handles this cleanly in [`resize_mouse`].
#[inline]
pub fn force_resize_mouse(ctx: &mut WmCtxX11, btn: MouseButton) {
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
pub fn resize_aspect_mouse(ctx: &mut WmCtx, win: WindowId, btn: MouseButton) {
    match ctx {
        WmCtx::X11(ctx_x11) => resize_aspect_mouse_x11(ctx_x11, win, btn),
        WmCtx::Wayland(wl) => {
            // Same approach as resize_mouse_from_cursor: use the current
            // pointer position to pick a direction and activate
            // HoverResizeDragState directly.
            let Some((ptr_x, ptr_y)) = wl.wayland.backend.pointer_location() else {
                return;
            };
            let Some(geo) = wl.core.g.clients.get(&win).map(|c| c.geo) else {
                return;
            };
            let hit_x = ptr_x - geo.x;
            let hit_y = ptr_y - geo.y;
            let dir = get_resize_direction(geo.w, geo.h, hit_x, hit_y);
            begin_wayland_super_resize(wl, win, btn, dir, geo, ptr_x, ptr_y);
        }
    }
}

pub fn resize_aspect_mouse_x11(ctx: &mut WmCtxX11, win: WindowId, btn: MouseButton) {
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

    with_wm_ctx_x11(ctx, |ctx| {
        crate::layouts::restack(ctx, ctx.g().selected_monitor_id())
    });

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

    let _rate = refresh_rate(ctx);
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

                if let Some((client_geo, sh, min_aspect, max_aspect)) = ctx
                    .core
                    .g
                    .clients
                    .get(&win)
                    .map(|c| (c.geo, c.size_hints.clone(), c.min_aspect, c.max_aspect))
                {
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

                    with_wm_ctx_x11(ctx, |ctx| {
                        resize(
                            ctx,
                            win,
                            &Rect {
                                x: client_geo.x,
                                y: client_geo.y,
                                w: nw,
                                h: nh,
                            },
                            true,
                        );
                    });
                }
            }

            _ => {}
        }
    }

    ungrab(ctx);
    with_wm_ctx_x11(ctx, |ctx| handle_client_monitor_switch(ctx, win));
}

// `hover_resize_mouse` and `is_in_resize_border` live in `super::hover`.
