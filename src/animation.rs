use crate::constants::animation::*;
use crate::contexts::{CoreCtx, WmCtx};
use crate::types::*;
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

pub fn ease_out_cubic(t: f64) -> f64 {
    let t = t - 1.0;
    1.0 + t * t * t
}

fn current_client_rect(core: &CoreCtx, win: WindowId) -> Option<Rect> {
    core.globals()
        .clients
        .get(&win)
        .map(|c| if c.geo.is_valid() { c.geo } else { c.old_geo })
}

pub(crate) fn interpolated_rect(animation: &WindowAnimation, now: Instant) -> Rect {
    let elapsed = now.saturating_duration_since(animation.started_at);
    let progress = if animation.duration.is_zero() {
        1.0
    } else {
        (elapsed.as_secs_f64() / animation.duration.as_secs_f64()).min(1.0)
    };
    let eased = ease_out_cubic(progress);

    let x = animation.from.x as f64 + (animation.to.x - animation.from.x) as f64 * eased;
    let y = animation.from.y as f64 + (animation.to.y - animation.from.y) as f64 * eased;
    let w = animation.from.w as f64 + (animation.to.w - animation.from.w) as f64 * eased;
    let h = animation.from.h as f64 + (animation.to.h - animation.from.h) as f64 * eased;

    Rect {
        x: x.round() as i32,
        y: y.round() as i32,
        w: w.round() as i32,
        h: h.round() as i32,
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
pub fn cancel_animation(ctx: &mut WmCtx<'_>, win: WindowId) {
    match ctx {
        WmCtx::X11(x11) => {
            if let Some(anim) = x11.x11_runtime.window_animations.remove(&win) {
                crate::contexts::WmCtx::X11(x11.reborrow()).resize_client(win, anim.to);
            }
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
            x11.x11_runtime.window_animations.insert(
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
    let current_tag = ctx.core().globals().selected_monitor().current_tag;
    crate::tags::view::scroll_view(ctx, dir);

    let monitor = ctx.core().globals().selected_monitor();
    if monitor.current_tag == current_tag {
        return;
    }

    let Some(win) = monitor.sel else {
        return;
    };

    let Some(client) = ctx.core().globals().clients.get(&win).cloned() else {
        return;
    };

    if !client.is_visible(monitor.selected_tags()) || client.is_true_fullscreen() {
        return;
    }

    let target = client.geo;
    let start_x = match dir {
        Direction::Left | Direction::Up => {
            monitor.monitor_rect.x - target.w - client.border_width * 2
        }
        Direction::Right | Direction::Down => {
            monitor.monitor_rect.x + monitor.monitor_rect.w + client.border_width * 2
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
