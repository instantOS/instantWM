use std::time::{Duration, Instant};

use crate::animation::{MoveResizeMode, WindowAnimation};
use crate::constants::animation::{
    DISTANCE_THRESHOLD, FRAME_SLEEP_MICROS, X11_ANIM_FULL_THRESHOLD, X11_ANIM_REDUCE_THRESHOLD,
};
use crate::contexts::WmCtx;
use crate::types::{Rect, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;

#[derive(Clone, Copy, Debug)]
pub enum GeometryReason {
    Direct,
    Layout,
    Spawn,
    Visibility,
    TagSwitch,
    Fullscreen,
    Interactive,
    Other(&'static str),
}

#[derive(Clone, Copy, Debug)]
pub struct GeometryRequest {
    pub win: WindowId,
    pub target: Rect,
    pub mode: MoveResizeMode,
    pub frames: i32,
    pub reason: GeometryReason,
}

impl GeometryRequest {
    pub fn immediate(win: WindowId, target: Rect, reason: GeometryReason) -> Self {
        Self {
            win,
            target,
            mode: MoveResizeMode::Immediate,
            frames: 0,
            reason,
        }
    }
}

fn current_client_rect(ctx: &WmCtx<'_>, win: WindowId) -> Option<Rect> {
    ctx.core()
        .globals()
        .clients
        .get(&win)
        .map(|c| if c.geo.is_valid() { c.geo } else { c.old_geo })
}

fn monitor_size_for_client(ctx: &WmCtx<'_>, win: WindowId) -> (i32, i32) {
    ctx.core()
        .globals()
        .clients
        .get(&win)
        .and_then(|c| ctx.core().globals().monitors.get(c.monitor_id))
        .map(|m| (m.monitor_rect.w, m.monitor_rect.h))
        .unwrap_or((
            ctx.core().globals().cfg.screen_width,
            ctx.core().globals().cfg.screen_height,
        ))
}

fn final_rect_for_target(target: Rect, mon_w: i32, mon_h: i32) -> Rect {
    Rect {
        x: target.x,
        y: target.y,
        w: target.w.min(mon_w).max(1),
        h: target.h.min(mon_h).max(1),
    }
}

fn animation_duration(frames: i32) -> Duration {
    Duration::from_micros(FRAME_SLEEP_MICROS * frames.max(0) as u64)
}

fn effective_animation_frames(active_count: usize, frames: i32) -> i32 {
    if active_count >= X11_ANIM_REDUCE_THRESHOLD {
        0
    } else if active_count >= X11_ANIM_FULL_THRESHOLD {
        (frames / 2).max(2)
    } else {
        frames
    }
}

fn active_animation_count(ctx: &WmCtx<'_>) -> usize {
    match ctx {
        WmCtx::X11(x11) => x11.x11_runtime.window_animations.len(),
        WmCtx::Wayland(wl) => wl
            .wayland
            .backend
            .with_state(|state| state.active_window_animation_count())
            .unwrap_or(0),
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

fn should_preserve_inflight_animation(ctx: &WmCtx<'_>, win: WindowId, target: Rect) -> bool {
    match ctx {
        WmCtx::X11(x11) => x11
            .x11_runtime
            .window_animations
            .get(&win)
            .is_some_and(|anim| anim.to == target),
        WmCtx::Wayland(wl) => {
            let border_width = wl
                .core
                .globals()
                .clients
                .get(&win)
                .map(|c| c.border_width())
                .unwrap_or(0);
            let requested_surface_target = Rect {
                x: target.x + border_width,
                y: target.y + border_width,
                w: target.w,
                h: target.h,
            };
            wl.wayland
                .backend
                .with_state(|state| {
                    state
                        .window_animation_target(win)
                        .is_some_and(|anim| anim.to == requested_surface_target)
                })
                .unwrap_or(false)
        }
    }
}

pub fn request(ctx: &mut WmCtx<'_>, request: GeometryRequest) {
    let _reason = request.reason;
    if !request.target.is_valid() {
        return;
    }

    let (mon_w, mon_h) = monitor_size_for_client(ctx, request.win);
    let final_rect = final_rect_for_target(request.target, mon_w, mon_h);

    match request.mode {
        MoveResizeMode::Immediate => {
            if !should_preserve_inflight_animation(ctx, request.win, final_rect) {
                crate::animation::cancel_animation(ctx, request.win);
            }
            ctx.apply_client_geometry_authoritative(request.win, final_rect);
        }
        MoveResizeMode::AnimateTo | MoveResizeMode::AnimateFrom(_) => {
            crate::animation::cancel_animation(ctx, request.win);

            let from = match request.mode {
                MoveResizeMode::AnimateTo => current_client_rect(ctx, request.win),
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
                if ctx.client(request.win).is_some_and(|c| c.geo != final_rect) {
                    ctx.apply_client_geometry_authoritative(request.win, final_rect);
                }
                return;
            }

            let animated = ctx.core().globals().behavior.animated;
            let effective_frames =
                effective_animation_frames(active_animation_count(ctx), request.frames);

            if !animated || effective_frames <= 0 {
                ctx.apply_client_geometry_authoritative(request.win, final_rect);
                return;
            }

            let dist_moved = (final_rect.x - from.x).abs() > DISTANCE_THRESHOLD
                || (final_rect.y - from.y).abs() > DISTANCE_THRESHOLD
                || (final_rect.w - from.w).abs() > DISTANCE_THRESHOLD
                || (final_rect.h - from.h).abs() > DISTANCE_THRESHOLD;
            if !dist_moved {
                ctx.apply_client_geometry_authoritative(request.win, final_rect);
                return;
            }

            if request.mode == MoveResizeMode::AnimateTo {
                ctx.apply_client_geometry_authoritative(request.win, final_rect);
            }

            enqueue_window_animation(ctx, request.win, from, final_rect, effective_frames);
        }
    }
}
