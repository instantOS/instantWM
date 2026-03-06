use crate::backend::x11::X11BackendRef;
use crate::backend::BackendOps;
use crate::constants::animation::*;
use crate::contexts::{CoreCtx, WmCtx, WmCtxWayland};
use crate::floating::{change_snap, SnapDir};
use crate::globals::X11RuntimeConfig;
use crate::tags::view::scroll_view;
use crate::types::*;
use std::thread;
use std::time::Duration;
use x11rb::connection::Connection;

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
        .and_then(|c| c.monitor_id)
        .and_then(|monitor_id| core.g.monitor(monitor_id))
        .map(|m| (m.monitor_rect.w, m.monitor_rect.h))
        .unwrap_or((0, 0))
}

fn clamp_to_monitor(target_w: i32, target_h: i32, mon_w: i32, mon_h: i32) -> (i32, i32) {
    let actual_w = if target_w > mon_w - 4 {
        mon_w - 4
    } else {
        target_w
    };
    let actual_h = if target_h > mon_h - 4 {
        mon_h - 4
    } else {
        target_h
    };
    (actual_w, actual_h)
}

fn final_rect(
    rect: &Rect,
    start_rect: &Rect,
    actual_w: i32,
    actual_h: i32,
    reset_pos: i32,
) -> Rect {
    let (x, y) = if reset_pos != 0 {
        (start_rect.x, start_rect.y)
    } else {
        (rect.x, rect.y)
    };
    Rect {
        x,
        y,
        w: actual_w,
        h: actual_h,
    }
}

fn try_resize_x11(core: &mut CoreCtx, x11: &X11BackendRef, win: WindowId, rect: &Rect) {
    if rect.is_valid() {
        crate::client::resize_client_x11(core, x11, win, rect);
    }
}

pub fn animate_client_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_cfg: &X11RuntimeConfig,
    win: WindowId,
    rect: &Rect,
    frames: i32,
    reset_pos: i32,
) {
    // Handled below by !ctx.g_mut().animated or frames <= 0 check.

    let start_rect = match get_start_rect(core, win, reset_pos) {
        Some(r) => r,
        None => return,
    };

    let target_w = if rect.w != 0 { rect.w } else { start_rect.w };
    let target_h = if rect.h != 0 { rect.h } else { start_rect.h };

    let (mon_w, mon_h) = get_monitor_size(core, win);
    let (actual_w, actual_h) = clamp_to_monitor(target_w, target_h, mon_w, mon_h);

    if !core.g.animated || frames <= 0 {
        try_resize_x11(
            core,
            x11,
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

    let effective_frames = if !x11_cfg.xlibdisplay.0.is_null() {
        let queued_events = unsafe {
            crate::drw::XEventsQueued(
                x11_cfg.xlibdisplay.0 as *mut std::os::raw::c_void,
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
        try_resize_x11(core, x11, win, &final_rect);
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
                    core,
                    x11,
                    x11_cfg,
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
                    core,
                    x11,
                    win,
                    &Rect {
                        x: step_x,
                        y: step_y,
                        w: actual_w,
                        h: actual_h,
                    },
                );
                let _ = x11.conn.flush();
                thread::sleep(Duration::from_micros(FRAME_SLEEP_MICROS));
            }
        }
    }

    try_resize_x11(core, x11, win, &final_rect);
    let _ = x11.conn.flush();
}

pub fn check_animate_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_cfg: &X11RuntimeConfig,
    win: WindowId,
    rect: &Rect,
    frames: i32,
    reset_pos: i32,
) {
    if let Some(client) = core.g.clients.get(&win) {
        let should_animate = client.geo.x != rect.x
            || client.geo.y != rect.y
            || client.geo.w != rect.w
            || client.geo.h != rect.h;
        if should_animate {
            animate_client_x11(core, x11, x11_cfg, win, rect, frames, reset_pos);
        }
    }
}

pub fn anim_scroll(ctx: &mut WmCtx, dir: Direction) {
    let sel_mon = ctx.g().selected_monitor_id();

    let (_is_floating, has_tiling, current_tag) = {
        let mon = ctx.g().selected_monitor();
        let is_floating = mon
            .sel
            .and_then(|w| ctx.g().clients.get(&w).map(|c| c.isfloating))
            .unwrap_or(false);
        let has_tiling = mon.is_tiling_layout();
        let current_tag = mon.current_tag as u32;
        (is_floating, has_tiling, current_tag)
    };

    let WmCtx::X11(ctx_x11) = ctx else {
        return;
    };

    if has_tiling {
        let focus_dir = match dir {
            Direction::Left => Direction::Up,
            Direction::Right => Direction::Down,
            Direction::Up => Direction::Up,
            Direction::Down => Direction::Down,
        };
        let mut target = None;
        crate::focus::focus_direction(&ctx_x11.core, focus_dir, |win| target = win);
        if let Some(win) = target {
            crate::focus::focus_soft(&mut WmCtx::X11(ctx_x11.reborrow()), Some(win));
        }
        return;
    }

    if !has_tiling {
        if let Some(selected_window) = ctx_x11.core.selected_client() {
            let snap_dir = match dir {
                Direction::Right => SnapDir::Right,
                Direction::Left => SnapDir::Left,
                Direction::Up => SnapDir::Up,
                Direction::Down => SnapDir::Down,
            };
            change_snap(
                &mut WmCtx::X11(ctx_x11.reborrow()),
                selected_window,
                snap_dir,
            );
        }
        return;
    }

    if current_tag == 0 {
        return;
    }

    if dir == Direction::Left && current_tag == 1 {
        return;
    }

    if dir == Direction::Right && current_tag >= MAX_TAG_NUMBER {
        return;
    }

    let animated = ctx_x11.core.g.animated;
    if animated {
        let modifier: i32 = match dir {
            Direction::Right => 1,
            Direction::Left => -1,
            Direction::Up => -1,
            Direction::Down => 1,
        };
        let target = current_tag + modifier as u32;
        check_client_on_target_tag(&ctx_x11.core.g, sel_mon, target);
    }

    match dir {
        Direction::Right => scroll_view(ctx, Direction::Right),
        Direction::Left => scroll_view(ctx, Direction::Left),
        Direction::Up => scroll_view(ctx, Direction::Left),
        Direction::Down => scroll_view(ctx, Direction::Right),
    }
}

pub fn animate_client(ctx: &mut WmCtx, win: WindowId, rect: &Rect, frames: i32, reset_pos: i32) {
    match ctx {
        WmCtx::X11(ref mut x11_ctx) => {
            let x11 = &x11_ctx.x11;
            let x11_runtime = x11_ctx.x11_runtime();
            animate_client_x11(
                &mut x11_ctx.core,
                x11,
                x11_runtime,
                win,
                rect,
                frames,
                reset_pos,
            )
        }
        WmCtx::Wayland(ref mut wl_ctx) => {
            animate_client_wayland(wl_ctx, win, rect, frames, reset_pos)
        }
    }
}

pub fn check_animate(ctx: &mut WmCtx, win: WindowId, rect: &Rect, frames: i32, reset_pos: i32) {
    match ctx {
        WmCtx::X11(ref mut x11_ctx) => {
            let x11 = &x11_ctx.x11;
            let x11_runtime = x11_ctx.x11_runtime();
            check_animate_x11(
                &mut x11_ctx.core,
                x11,
                x11_runtime,
                win,
                rect,
                frames,
                reset_pos,
            )
        }
        WmCtx::Wayland(ref mut wl_ctx) => {
            let should_animate = wl_ctx
                .core
                .g
                .clients
                .get(&win)
                .map(|c| c.geo != *rect)
                .unwrap_or(false);
            if should_animate {
                animate_client_wayland(wl_ctx, win, rect, frames, reset_pos);
            }
        }
    }
}

fn animate_client_wayland(
    ctx: &mut WmCtxWayland,
    win: WindowId,
    rect: &Rect,
    frames: i32,
    reset_pos: i32,
) {
    let start_rect =
        match ctx
            .core
            .g
            .clients
            .get(&win)
            .map(|c| if reset_pos != 0 { c.geo } else { c.old_geo })
        {
            Some(r) => r,
            None => return,
        };
    let target_w = if rect.w != 0 { rect.w } else { start_rect.w };
    let target_h = if rect.h != 0 { rect.h } else { start_rect.h };
    let final_rect = Rect {
        x: if reset_pos != 0 { start_rect.x } else { rect.x },
        y: if reset_pos != 0 { start_rect.y } else { rect.y },
        w: target_w.max(1),
        h: target_h.max(1),
    };

    if let Some(c) = ctx.core.g.clients.get_mut(&win) {
        c.old_geo = c.geo;
        c.geo = final_rect;
    }

    if frames <= 0 || !ctx.core.g.animated {
        let was_animated = ctx.core.g.animated;
        ctx.core.g.animated = false;
        ctx.backend.resize_window(win, final_rect);
        ctx.core.g.animated = was_animated;
    } else {
        ctx.backend.resize_window(win, final_rect);
    }
}

fn check_client_on_target_tag(globals: &crate::globals::Globals, sel_mon: MonitorId, target: u32) {
    if let Some(mon) = globals.monitor(sel_mon) {
        for (_c_win, c) in mon.iter_clients(&globals.clients) {
            let _has_client_on_tag = (c.tags & (1 << (target - 1))) != 0 && !c.isfloating;
        }
    }
}
