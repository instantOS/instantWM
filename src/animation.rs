use crate::backend::x11::X11RuntimeConfig;
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

/// Backend-local visual state for the manual-layout preview.
///
/// The core owns the authoritative target rectangle. Backends own this small
/// projection because they already own animation clocks and redraw scheduling.
/// Retargeting starts at the currently displayed rectangle, so repeated key
/// presses remain continuous instead of jumping back to an obsolete origin.
#[derive(Clone, Debug, Default)]
pub struct LayoutPreviewAnimation {
    displayed: Option<Rect>,
    transition: Option<WindowAnimation>,
}

impl LayoutPreviewAnimation {
    pub fn set_target(
        &mut self,
        target: Option<Rect>,
        animate: bool,
        duration: Duration,
        now: Instant,
    ) -> Option<Rect> {
        let Some(target) = target else {
            self.displayed = None;
            self.transition = None;
            return None;
        };

        let from = self.tick(now);
        if !animate || from.is_none() || from == Some(target) {
            self.displayed = Some(target);
            self.transition = None;
            return self.displayed;
        }

        let from = from.expect("an animated preview has a displayed origin");
        self.transition = Some(WindowAnimation {
            from,
            to: target,
            started_at: now,
            duration,
        });
        self.displayed
    }

    pub fn tick(&mut self, now: Instant) -> Option<Rect> {
        let Some(transition) = self.transition.as_ref() else {
            return self.displayed;
        };
        let tick = transition.tick(now);
        self.displayed = Some(tick.rect);
        if tick.done {
            self.transition = None;
        }
        self.displayed
    }

    pub fn displayed(&self) -> Option<Rect> {
        self.displayed
    }

    pub fn is_active(&self) -> bool {
        self.transition.is_some()
    }
}

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

#[cfg(test)]
mod layout_preview_tests {
    use super::*;

    #[test]
    fn preview_retargets_from_its_current_visual_rectangle() {
        let start = Instant::now();
        let duration = Duration::from_millis(100);
        let mut preview = LayoutPreviewAnimation::default();
        let first = Rect::new(0, 0, 100, 100);
        let second = Rect::new(100, 0, 100, 100);
        let third = Rect::new(200, 0, 100, 100);

        assert_eq!(
            preview.set_target(Some(first), true, duration, start),
            Some(first)
        );
        preview.set_target(Some(second), true, duration, start);
        assert_eq!(
            preview.tick(start + Duration::from_millis(50)).unwrap().x,
            88
        );

        let displayed = preview.displayed().unwrap();
        assert_eq!(
            preview.set_target(
                Some(third),
                true,
                duration,
                start + Duration::from_millis(50),
            ),
            Some(displayed),
        );
        assert!(preview.is_active());
    }

    #[test]
    fn hiding_preview_cancels_its_transition() {
        let now = Instant::now();
        let mut preview = LayoutPreviewAnimation::default();
        preview.set_target(Some(Rect::new(0, 0, 100, 100)), false, Duration::ZERO, now);
        preview.set_target(
            Some(Rect::new(100, 0, 100, 100)),
            true,
            Duration::from_millis(100),
            now,
        );

        assert_eq!(preview.set_target(None, true, Duration::ZERO, now), None);
        assert!(!preview.is_active());
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
pub fn drop_x11_animation(x11_runtime: &mut X11RuntimeConfig, win: WindowId) {
    let _ = x11_runtime.take_window_animation(win);
}

pub fn cancel_animation(ctx: &mut WmCtx<'_>, win: WindowId) {
    match ctx {
        WmCtx::X11(x11) => {
            cancel_x11_animation(x11, win);
        }
        WmCtx::Wayland(wl) => {
            let _ = wl
                .wayland
                .with_state(|state| state.cancel_window_animation(win));
        }
    }
}

/// Take an in-flight animation and return its rectangle at `now` without
/// snapping to the obsolete target. This is the correct starting point when a
/// live interaction retargets a moving window.
pub(crate) fn take_current_animation_rect(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    now: Instant,
) -> Option<Rect> {
    match ctx {
        WmCtx::X11(x11) => x11
            .x11_runtime
            .take_window_animation(win)
            .map(|animation| animation.tick(now).rect),
        WmCtx::Wayland(wl) => wl
            .wayland
            .with_state(|state| state.take_current_window_animation_rect(win, now))
            .flatten(),
    }
}

pub fn scroll_view_with_slide(ctx: &mut WmCtx, dir: HorizontalDirection) {
    let old_selected_tags = ctx.core().model().expect_selected_monitor().selected_tags();
    let Some(selmon_id) = crate::tags::view::scroll_view_for_slide(ctx, dir) else {
        return;
    };

    crate::layouts::arrange(ctx, Some(selmon_id));

    let (monitor_rect, selected_tags, clients) = {
        let Some(monitor) = ctx.core().model().monitor(selmon_id) else {
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
        let Some(client) = ctx.core().model().client(win).cloned() else {
            continue;
        };
        if !client.is_visible(selected_tags)
            || client.is_visible(old_selected_tags)
            || client.mode.is_true_fullscreen()
            || !client.geo.is_valid()
        {
            continue;
        }
        animation_targets.push((win, client.geo, client.border_width));
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
