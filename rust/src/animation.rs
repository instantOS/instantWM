use crate::constants::animation::*;
use crate::contexts::WmCtx;
use crate::floating::{change_snap, SnapDir};
use crate::globals::get_globals;
use crate::monitor::is_current_layout_tiling;
use crate::tags::view::scroll_view;
use crate::types::*;
use std::thread;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::Window;

const QUEUED_ALREADY: std::os::raw::c_int = 0;

pub fn ease_out_cubic(t: f64) -> f64 {
    let t = t - 1.0;
    1.0 + t * t * t
}

fn get_start_rect(win: Window, reset_pos: i32) -> Option<Rect> {
    let globals = get_globals();
    globals
        .clients
        .get(&win)
        .map(|c| if reset_pos != 0 { c.geo } else { c.old_geo })
}

fn get_monitor_size(win: Window) -> (i32, i32) {
    let globals = get_globals();
    globals
        .clients
        .get(&win)
        .and_then(|c| c.mon_id)
        .and_then(|mon_id| globals.monitors.get(mon_id))
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

fn try_resize(ctx: &mut WmCtx, win: Window, rect: &Rect) {
    if rect.is_valid() {
        crate::client::resize_client(ctx, win, rect);
    }
}

pub fn animate_client(ctx: &mut WmCtx, win: Window, rect: &Rect, frames: i32, reset_pos: i32) {
    if frames <= 0 {
        return;
    }

    let start_rect = match get_start_rect(win, reset_pos) {
        Some(r) => r,
        None => return,
    };

    let target_w = if rect.w != 0 { rect.w } else { start_rect.w };
    let target_h = if rect.h != 0 { rect.h } else { start_rect.h };

    let Some(ref conn) = ctx.x11.conn else {
        return;
    };

    let (mon_w, mon_h) = get_monitor_size(win);
    let (actual_w, actual_h) = clamp_to_monitor(target_w, target_h, mon_w, mon_h);

    if !ctx.g.animated {
        try_resize(
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

    let queued_events = unsafe {
        crate::drw::XEventsQueued(
            ctx.g.cfg.xlibdisplay.0 as *mut std::os::raw::c_void,
            QUEUED_ALREADY,
        )
    };
    let effective_frames = if queued_events > QUEUE_SKIP_THRESHOLD {
        0
    } else if queued_events > QUEUE_REDUCE_THRESHOLD {
        (frames / 2).max(1)
    } else {
        frames
    };

    let final_rect = final_rect(rect, &start_rect, actual_w, actual_h, reset_pos);

    if effective_frames == 0 {
        try_resize(ctx, win, &final_rect);
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
                animate_client(
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
                try_resize(
                    ctx,
                    win,
                    &Rect {
                        x: step_x,
                        y: step_y,
                        w: actual_w,
                        h: actual_h,
                    },
                );
                let _ = conn.flush();
                thread::sleep(Duration::from_micros(FRAME_SLEEP_MICROS));
            }
        }
    }

    try_resize(ctx, win, &final_rect);
}

pub fn check_animate(ctx: &mut WmCtx, win: Window, rect: &Rect, frames: i32, reset_pos: i32) {
    if let Some(client) = ctx.g.clients.get(&win) {
        let should_animate = client.geo.x != rect.x
            || client.geo.y != rect.y
            || client.geo.w != rect.w
            || client.geo.h != rect.h;
        if should_animate {
            animate_client(ctx, win, rect, frames, reset_pos);
        }
    }
}

pub fn up_scale_client(ctx: &mut WmCtx, win: Window) {
    crate::client::scale_client(ctx, win, 110);
}

pub fn down_scale_client(ctx: &mut WmCtx, win: Window) {
    crate::client::scale_client(ctx, win, 90);
}

pub fn anim_scroll(ctx: &mut WmCtx, dir: Direction) {
    let sel_mon = ctx.g.selmon;

    let (_is_floating, has_tiling, current_tag) = {
        let mon = match ctx.g.monitors.get(sel_mon) {
            Some(m) => m,
            None => return,
        };
        let is_floating = mon
            .sel
            .and_then(|w| ctx.g.clients.get(&w).map(|c| c.isfloating))
            .unwrap_or(false);
        let has_tiling = is_current_layout_tiling(mon);
        let current_tag = mon.current_tag as u32;
        (is_floating, has_tiling, current_tag)
    };

    if has_tiling {
        let focus_dir = match dir {
            Direction::Left => Direction::Up,
            Direction::Right => Direction::Down,
            Direction::Up => Direction::Up,
            Direction::Down => Direction::Down,
        };
        let mut target = None;
        crate::focus::focus_direction(ctx, focus_dir, |win| target = win);
        if let Some(win) = target {
            crate::focus::focus(ctx, Some(win));
        }
        return;
    }

    if !has_tiling {
        if let Some(sel_win) = ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.sel) {
            let snap_dir = match dir {
                Direction::Right => SnapDir::Right,
                Direction::Left => SnapDir::Left,
                Direction::Up => SnapDir::Up,
                Direction::Down => SnapDir::Down,
            };
            change_snap(ctx, sel_win, snap_dir);
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

    let animated = ctx.g.animated;
    if animated {
        let modifier: i32 = match dir {
            Direction::Right => 1,
            Direction::Left => -1,
            Direction::Up => -1,
            Direction::Down => 1,
        };
        let target = current_tag + modifier as u32;
        check_client_on_target_tag(sel_mon, target);
    }

    match dir {
        Direction::Right => scroll_view(ctx, Direction::Right),
        Direction::Left => scroll_view(ctx, Direction::Left),
        Direction::Up => scroll_view(ctx, Direction::Left),
        Direction::Down => scroll_view(ctx, Direction::Right),
    }
}

fn check_client_on_target_tag(sel_mon: MonitorId, target: u32) {
    let globals = get_globals();
    if let Some(mon) = globals.monitors.get(sel_mon) {
        let mut current = mon.clients;
        while let Some(c_win) = current {
            if let Some(c) = globals.clients.get(&c_win) {
                let _has_client_on_tag = (c.tags & (1 << (target - 1))) != 0 && !c.isfloating;
                current = c.next;
            } else {
                break;
            }
        }
    }
}
