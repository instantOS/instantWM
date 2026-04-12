use crate::constants::animation::*;
use crate::contexts::WmCtx;
use crate::types::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};

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

/// Cancel an in-flight X11 animation by snapping directly to its target.
pub fn cancel_x11_animation(ctx: &mut crate::contexts::WmCtxX11<'_>, win: WindowId) {
    if let Some(anim) = ctx.x11_runtime.take_window_animation(win) {
        crate::contexts::WmCtx::X11(ctx.reborrow())
            .apply_client_geometry_authoritative(win, anim.to);
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

pub fn move_resize_client(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    target: &Rect,
    mode: MoveResizeMode,
    frames: i32,
) {
    crate::geometry::request(
        ctx,
        crate::geometry::GeometryRequest {
            win,
            target: *target,
            mode,
            frames,
            reason: crate::geometry::GeometryReason::Animation,
        },
    );
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
        let Some(client) = ctx.client(win).cloned() else {
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
