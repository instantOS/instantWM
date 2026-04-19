use crate::constants::animation::*;
use crate::contexts::WmCtx;
use crate::geometry::{GeometryApplyMode, MoveResizeOptions};
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

impl WindowAnimation {
    pub fn tick(&self, now: Instant) -> AnimationTick {
        let elapsed = now.saturating_duration_since(self.started_at);
        let raw_t = if self.duration.is_zero() {
            1.0
        } else {
            (elapsed.as_secs_f64() / self.duration.as_secs_f64()).clamp(0.0, 1.0)
        };
        let eased = ease_out_cubic(raw_t);

        let x = self.from.x as f64 + (self.to.x - self.from.x) as f64 * eased;
        let y = self.from.y as f64 + (self.to.y - self.from.y) as f64 * eased;
        let w = self.from.w as f64 + (self.to.w - self.from.w) as f64 * eased;
        let h = self.from.h as f64 + (self.to.h - self.from.h) as f64 * eased;

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
}

/// Cancel an in-flight X11 animation by snapping directly to its target.
pub fn cancel_x11_animation(ctx: &mut crate::contexts::WmCtxX11<'_>, win: WindowId) {
    if let Some(anim) = ctx.x11_runtime.take_window_animation(win) {
        let mut wmctx = crate::contexts::WmCtx::X11(ctx.reborrow());
        wmctx.set_geometry_impl(win, anim.to, GeometryApplyMode::Logical);
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

pub fn scroll_view_with_slide(ctx: &mut WmCtx, dir: HorizontalDirection) {
    let old_selected_tags = ctx.core().globals().selected_monitor().selected_tags();
    let Some(selmon_id) = crate::tags::view::scroll_view_for_slide(ctx, dir) else {
        return;
    };

    crate::layouts::arrange(ctx, Some(selmon_id));

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
        let Some(client) = ctx.core().client(win).cloned() else {
            continue;
        };
        if !client.is_visible(selected_tags)
            || client.is_visible(old_selected_tags)
            || client.mode.is_true_fullscreen()
            || !client.geo.is_valid()
        {
            continue;
        }
        animation_targets.push((win, client.geo, client.border_width()));
    }

    for (win, target, border_width) in animation_targets {
        let start_x = match dir {
            HorizontalDirection::Left => monitor_rect.x - target.w - border_width * 2,
            HorizontalDirection::Right => monitor_rect.x + monitor_rect.w + border_width * 2,
        };
        let start = Rect {
            x: start_x,
            y: target.y,
            w: target.w,
            h: target.h,
        };
        ctx.move_resize(
            win,
            target,
            MoveResizeOptions::animate_from(start, DEFAULT_FRAME_COUNT),
        );
    }
}
