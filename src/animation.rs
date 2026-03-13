use crate::constants::animation::*;
use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
use crate::tags::view::scroll_view;
use crate::types::*;
use std::thread;
use std::time::Duration;
use x11rb::connection::Connection;

/// Backend-agnostic animation entry point.
///
/// On X11: performs smooth animation with easing.
/// On Wayland: immediately sets the geometry (Wayland handles transitions differently).
pub fn animate_client(ctx: &mut WmCtx, win: WindowId, rect: &Rect, frames: i32, reset_pos: i32) {
    match ctx {
        WmCtx::X11(ctx_x11) => animate_client_x11(ctx_x11, win, rect, frames, reset_pos),
        WmCtx::Wayland(_ctx_wayland) => {
            // Wayland: no smooth animation, just resize immediately
            ctx.resize_client(win, *rect);
        }
    }
}

/// Backend-agnostic check and animate.
pub fn check_animate(ctx: &mut WmCtx, win: WindowId, rect: &Rect, frames: i32, reset_pos: i32) {
    match ctx {
        WmCtx::X11(ctx_x11) => check_animate_x11(ctx_x11, win, rect, frames, reset_pos),
        WmCtx::Wayland(_ctx_wayland) => {
            // Check if geometry actually changed
            let should_animate = ctx
                .g()
                .clients
                .get(&win)
                .map(|client| {
                    client.geo.x != rect.x
                        || client.geo.y != rect.y
                        || client.geo.w != rect.w
                        || client.geo.h != rect.h
                })
                .unwrap_or(false);
            if should_animate {
                ctx.resize_client(win, *rect);
            }
        }
    }
}

const QUEUED_ALREADY: std::os::raw::c_int = 0;

pub fn ease_out_cubic(t: f64) -> f64 {
    let t = t - 1.0;
    1.0 + t * t * t
}

fn get_start_rect(core: &CoreCtx, win: WindowId, reset_pos: i32) -> Option<Rect> {
    core.g
        .clients
        .get(&win)
        .map(|c| if reset_pos != 0 { c.geo } else { c.old_geo })
}

fn get_monitor_size(core: &CoreCtx, win: WindowId) -> (i32, i32) {
    core.g
        .clients
        .get(&win)
        .and_then(|c| core.g.monitors.get(c.monitor_id))
        .map(|m| (m.monitor_rect.w, m.monitor_rect.h))
        .unwrap_or((core.g.cfg.screen_width, core.g.cfg.screen_height))
}

fn clamp_to_monitor(target_w: i32, target_h: i32, mon_w: i32, mon_h: i32) -> (i32, i32) {
    (target_w.min(mon_w), target_h.min(mon_h))
}

fn final_rect(
    rect: &Rect,
    start_rect: &Rect,
    actual_w: i32,
    actual_h: i32,
    reset_pos: i32,
) -> Rect {
    let (x, y) = if reset_pos != 0 {
        (rect.x, rect.y)
    } else {
        (start_rect.x, start_rect.y)
    };
    Rect {
        x,
        y,
        w: actual_w,
        h: actual_h,
    }
}

fn try_resize_x11(ctx: &mut WmCtxX11<'_>, win: WindowId, rect: &Rect) {
    if rect.is_valid() {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        wm_ctx.resize_client(win, *rect);
    }
}

pub fn animate_client_x11(
    ctx: &mut WmCtxX11<'_>,
    win: WindowId,
    rect: &Rect,
    frames: i32,
    reset_pos: i32,
) {
    // Handled below by !ctx.g_mut().animated or frames <= 0 check.

    let start_rect = match get_start_rect(&ctx.core, win, reset_pos) {
        Some(r) => r,
        None => return,
    };

    let target_w = if rect.w != 0 { rect.w } else { start_rect.w };
    let target_h = if rect.h != 0 { rect.h } else { start_rect.h };

    let (mon_w, mon_h) = get_monitor_size(&ctx.core, win);
    let (actual_w, actual_h) = clamp_to_monitor(target_w, target_h, mon_w, mon_h);

    if !ctx.core.g.animated || frames <= 0 {
        try_resize_x11(
            ctx,
            win,
            &Rect {
                x: rect.x,
                y: rect.y,
                w: actual_w,
                h: actual_h,
            },
        );
        return;
    }

    let effective_frames = if !ctx.x11_runtime.xlibdisplay.0.is_null() {
        let queued_events = unsafe {
            crate::drw::XEventsQueued(
                ctx.x11_runtime.xlibdisplay.0 as *mut std::os::raw::c_void,
                QUEUED_ALREADY,
            )
        };
        if queued_events > QUEUE_SKIP_THRESHOLD {
            0
        } else if queued_events > QUEUE_REDUCE_THRESHOLD {
            (frames / 2).max(1)
        } else {
            frames
        }
    } else {
        frames
    };

    let final_rect = final_rect(rect, &start_rect, actual_w, actual_h, reset_pos);

    if effective_frames == 0 {
        try_resize_x11(ctx, win, &final_rect);
        return;
    }

    let dx = (rect.x - start_rect.x) as f64;
    let dy = (rect.y - start_rect.y) as f64;

    let dist_moved = (start_rect.x - rect.x).abs() > DISTANCE_THRESHOLD
        || (start_rect.y - rect.y).abs() > DISTANCE_THRESHOLD
        || (actual_w - start_rect.w).abs() > DISTANCE_THRESHOLD
        || (actual_h - start_rect.h).abs() > DISTANCE_THRESHOLD;

    if dist_moved {
        if rect.x == start_rect.x
            && rect.y == start_rect.y
            && start_rect.w < mon_w - MONITOR_WIDTH_THRESHOLD
        {
            let delta_w = actual_w - start_rect.w;
            let delta_h = actual_h - start_rect.h;
            if delta_w != 0 || delta_h != 0 {
                animate_client_x11(
                    ctx,
                    win,
                    &Rect {
                        x: start_rect.x + delta_w,
                        y: start_rect.y + delta_h,
                        w: actual_w,
                        h: actual_h,
                    },
                    effective_frames,
                    0,
                );
            }
        } else {
            for time in 1..=effective_frames {
                let progress = ease_out_cubic(time as f64 / effective_frames as f64);
                let step_x = (start_rect.x as f64 + progress * dx) as i32;
                let step_y = (start_rect.y as f64 + progress * dy) as i32;
                try_resize_x11(
                    ctx,
                    win,
                    &Rect {
                        x: step_x,
                        y: step_y,
                        w: actual_w,
                        h: actual_h,
                    },
                );
                let _ = ctx.x11.conn.flush();
                thread::sleep(Duration::from_micros(FRAME_SLEEP_MICROS));
            }
        }
    }

    try_resize_x11(ctx, win, &final_rect);
    let _ = ctx.x11.conn.flush();
}

pub fn check_animate_x11(
    ctx: &mut WmCtxX11<'_>,
    win: WindowId,
    rect: &Rect,
    frames: i32,
    reset_pos: i32,
) {
    if let Some(client) = ctx.core.g.clients.get(&win) {
        let should_animate = client.geo.x != rect.x
            || client.geo.y != rect.y
            || client.geo.w != rect.w
            || client.geo.h != rect.h;
        if should_animate {
            animate_client_x11(ctx, win, rect, frames, reset_pos);
        }
    }
}

pub fn anim_scroll(ctx: &mut WmCtx, dir: Direction) {
    let sel_mon = ctx.g().selected_monitor_id();

    let (has_tiling, current_tag) = {
        let mon = ctx.g().selected_monitor();
        let has_tiling = mon.is_tiling_layout();
        let current_tag = mon.current_tag as u32;
        (has_tiling, current_tag)
    };

    if has_tiling {
        crate::focus::direction_focus(ctx, dir);
    } else {
        scroll_view(ctx, dir);
    }

    let clients_to_animate: Vec<(WindowId, Rect)> = ctx
        .g()
        .clients
        .iter()
        .filter(|(_, client)| client.monitor_id == sel_mon && client.tags == current_tag)
        .map(|(id, client)| (*id, client.geo))
        .collect();
    for (id, rect) in clients_to_animate {
        animate_client(ctx, id, &rect, 10, 0);
    }
}
