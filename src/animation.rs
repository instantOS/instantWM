use crate::constants::animation::*;
use crate::contexts::{CoreCtx, WmCtx};
use crate::types::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;

#[derive(Clone, Debug)]
pub struct WindowAnimation {
    pub from: Rect,
    pub to: Rect,
    pub started_at: Instant,
    pub duration: Duration,
}

pub type WindowAnimations = HashMap<WindowId, WindowAnimation>;

pub fn ease_out_cubic(t: f64) -> f64 {
    let t = t - 1.0;
    1.0 + t * t * t
}

#[derive(Clone, Copy, Debug)]
pub struct AnimationTick {
    pub rect: Rect,
    pub done: bool,
}

fn current_client_rect(core: &CoreCtx, win: WindowId) -> Option<Rect> {
    core.globals()
        .clients
        .get(&win)
        .map(|c| if c.geo.is_valid() { c.geo } else { c.old_geo })
}

pub fn interpolate_animation_tick(animation: &WindowAnimation, now: Instant) -> AnimationTick {
    let elapsed = now.saturating_duration_since(animation.started_at);
    let raw_t = if animation.duration.is_zero() {
        1.0
    } else {
        (elapsed.as_secs_f64() / animation.duration.as_secs_f64()).clamp(0.0, 1.0)
    };
    let eased = ease_out_cubic(raw_t);

    let x = animation.from.x as f64 + (animation.to.x - animation.from.x) as f64 * eased;
    let y = animation.from.y as f64 + (animation.to.y - animation.from.y) as f64 * eased;
    let w = animation.from.w as f64 + (animation.to.w - animation.from.w) as f64 * eased;
    let h = animation.from.h as f64 + (animation.to.h - animation.from.h) as f64 * eased;

    AnimationTick {
        rect: Rect {
            x: x.round() as i32,
            y: y.round() as i32,
            w: w.round() as i32,
            h: h.round() as i32,
        },
        done: raw_t >= 1.0,
    }
}

fn get_monitor_size(core: &CoreCtx, win: WindowId) -> (i32, i32) {
    core.globals()
        .clients
        .get(&win)
        .and_then(|c| core.globals().monitors.get(c.monitor_id))
        .map(|m| (m.monitor_rect.w, m.monitor_rect.h))
        .unwrap_or((
            core.globals().cfg.screen_width,
            core.globals().cfg.screen_height,
        ))
}

fn final_rect(rect: &Rect, actual_w: i32, actual_h: i32) -> Rect {
    Rect {
        x: rect.x,
        y: rect.y,
        w: actual_w,
        h: actual_h,
    }
}

fn animation_duration(frames: i32) -> Duration {
    Duration::from_micros(FRAME_SLEEP_MICROS * frames.max(0) as u64)
}

fn effective_animation_frames(count: usize, frames: i32) -> i32 {
    if count >= X11_ANIM_REDUCE_THRESHOLD {
        0
    } else if count >= X11_ANIM_FULL_THRESHOLD {
        (frames / 2).max(2)
    } else {
        frames
    }
}

/// Cancel an in-flight animation for a single window, snapping it to the
/// animation target.  This ensures that `current_client_rect` (c.geo) is
/// authoritative before any new animation is started.
pub fn cancel_x11_animation(ctx: &mut crate::contexts::WmCtxX11<'_>, win: WindowId) {
    if let Some(anim) = ctx.x11_runtime.take_window_animation(win) {
        crate::contexts::WmCtx::X11(ctx.reborrow()).resize_client(win, anim.to);
    }
}

/// Drop an in-flight X11 animation without applying its final target.
pub fn drop_x11_animation(ctx: &mut crate::contexts::WmCtxX11<'_>, win: WindowId) {
    let _ = ctx.x11_runtime.take_window_animation(win);
}

pub fn cancel_animation(ctx: &mut WmCtx<'_>, win: WindowId) {
    match ctx {
        WmCtx::X11(x11) => {
            cancel_x11_animation(x11, win);
        }
        WmCtx::Wayland(wl) => {
            let _ = wl
                .wayland
                .backend
                .with_state(|state| state.cancel_window_animation(win));
        }
    }
}

fn enqueue_window_animation(ctx: &mut WmCtx<'_>, win: WindowId, from: Rect, to: Rect, frames: i32) {
    let duration = animation_duration(frames);
    match ctx {
        WmCtx::X11(x11) => {
            let x11_win: x11rb::protocol::xproto::Window = win.into();
            let _ = x11.x11.conn.configure_window(
                x11_win,
                &x11rb::protocol::xproto::ConfigureWindowAux::new()
                    .x(from.x)
                    .y(from.y)
                    .width(from.w.max(1) as u32)
                    .height(from.h.max(1) as u32),
            );
            let _ = x11.x11.conn.flush();
            x11.x11_runtime.insert_or_replace_window_animation(
                win,
                WindowAnimation {
                    from,
                    to,
                    started_at: Instant::now(),
                    duration,
                },
            );
        }
        WmCtx::Wayland(wl) => {
            let _ = wl.wayland.backend.with_state(|state| {
                state.set_window_target_rect(
                    win,
                    to,
                    crate::backend::wayland::compositor::window::animations::WindowMoveMode::AnimateFrom {
                        from,
                        duration,
                    },
                );
            });
        }
    }
}

fn sync_authoritative_client_rect(ctx: &mut WmCtx<'_>, win: WindowId, rect: Rect) {
    crate::client::sync_client_geometry(ctx.core_mut().globals_mut(), win, rect);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MoveResizeMode {
    /// The window is moving to a new position.  Cancelling snaps to target.
    /// Updates c.geo immediately.
    AnimateTo,
    /// Instant move, no animation.
    Immediate,
    /// Purely decorative: the window visually starts from the given position
    /// and animates back to where it already logically is.  Cancelling snaps
    /// to the original (current) position.  c.geo is NOT changed.
    AnimateFrom(Rect),
}

fn animate_rect_transition(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    target: &Rect,
    mode: MoveResizeMode,
    frames: i32,
) {
    if !target.is_valid() {
        return;
    }

    let (mon_w, mon_h) = get_monitor_size(ctx.core(), win);
    let final_rect = final_rect(
        target,
        target.w.min(mon_w).max(1),
        target.h.min(mon_h).max(1),
    );

    // Always cancel any existing animation first so that c.geo is
    // authoritative and no stale intermediate state leaks into the new
    // animation.
    cancel_animation(ctx, win);

    if mode == MoveResizeMode::Immediate {
        ctx.resize_client(win, final_rect);
        return;
    }

    let from = match mode {
        MoveResizeMode::AnimateTo => current_client_rect(ctx.core(), win),
        MoveResizeMode::Immediate => unreachable!(),
        MoveResizeMode::AnimateFrom(from) => Some(from),
    };
    let Some(from) = from else {
        return;
    };
    if !from.is_valid() {
        return;
    }

    if from == final_rect {
        if ctx.client(win).is_some_and(|c| c.geo != final_rect) {
            ctx.resize_client(win, final_rect);
        }
        return;
    }

    let animated = ctx.core().globals().behavior.animated;
    let effective_frames = match ctx {
        WmCtx::X11(x11) => {
            effective_animation_frames(x11.x11_runtime.window_animations.len(), frames)
        }
        WmCtx::Wayland(wl) => {
            let count = wl
                .wayland
                .backend
                .with_state(|state| state.active_window_animation_count())
                .unwrap_or(0);
            effective_animation_frames(count, frames)
        }
    };

    if !animated || effective_frames <= 0 {
        ctx.resize_client(win, final_rect);
        return;
    }

    let dist_moved = (final_rect.x - from.x).abs() > DISTANCE_THRESHOLD
        || (final_rect.y - from.y).abs() > DISTANCE_THRESHOLD
        || (final_rect.w - from.w).abs() > DISTANCE_THRESHOLD
        || (final_rect.h - from.h).abs() > DISTANCE_THRESHOLD;
    if !dist_moved {
        ctx.resize_client(win, final_rect);
        return;
    }

    if mode == MoveResizeMode::AnimateTo {
        sync_authoritative_client_rect(ctx, win, final_rect);
    }

    enqueue_window_animation(ctx, win, from, final_rect, effective_frames);
}

pub fn move_resize_client(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    target: &Rect,
    mode: MoveResizeMode,
    frames: i32,
) {
    animate_rect_transition(ctx, win, target, mode, frames);
}

pub fn scroll_view_with_slide(ctx: &mut WmCtx, dir: Direction) {
    let Some(selmon_id) = crate::tags::view::scroll_view_for_slide(ctx, dir) else {
        return;
    };

    let (monitor_rect, selected_tags, clients) = {
        let Some(monitor) = ctx.core().globals().monitor(selmon_id) else {
            return;
        };
        (
            monitor.monitor_rect,
            monitor.selected_tags(),
            monitor.clients.clone(),
        )
    };

    let mut animation_targets = Vec::new();
    for win in clients {
        let Some(client) = ctx.core().globals().clients.get(&win).cloned() else {
            continue;
        };
        if !client.is_visible(selected_tags)
            || client.is_true_fullscreen()
            || !client.geo.is_valid()
        {
            continue;
        }
        animation_targets.push((win, client.geo, client.border_width()));
    }

    for (win, target, border_width) in animation_targets {
        let start_x = match dir {
            Direction::Left | Direction::Up => monitor_rect.x - target.w - border_width * 2,
            Direction::Right | Direction::Down => {
                monitor_rect.x + monitor_rect.w + border_width * 2
            }
        };
        let start = Rect {
            x: start_x,
            y: target.y,
            w: target.w,
            h: target.h,
        };
        move_resize_client(
            ctx,
            win,
            &target,
            MoveResizeMode::AnimateFrom(start),
            DEFAULT_FRAME_COUNT,
        );
    }
}
