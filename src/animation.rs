use crate::backend::x11::X11RuntimeConfig;
use crate::constants::animation::*;
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
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
    /// Linear progress before easing, in the inclusive range `0.0..=1.0`.
    pub progress: f64,
    pub done: bool,
}

/// Interpolate a rectangle using the animation system's easing and rounding.
pub(crate) fn interpolate_rect(from: Rect, to: Rect, linear_progress: f64) -> Rect {
    let eased = ease_out_cubic(linear_progress.clamp(0.0, 1.0));
    let interpolate =
        |start: i32, end: i32| (f64::from(start) + f64::from(end - start) * eased).round() as i32;

    Rect::new(
        interpolate(from.x, to.x),
        interpolate(from.y, to.y),
        interpolate(from.w, to.w),
        interpolate(from.h, to.h),
    )
}

impl WindowAnimation {
    /// Sample the transition at an explicit linear progress value.
    ///
    /// Keeping interpolation here ensures animation playback and policy code
    /// that probes a future frame use exactly the same easing and rounding.
    pub fn sample(&self, linear_progress: f64) -> AnimationTick {
        let progress = linear_progress.clamp(0.0, 1.0);
        AnimationTick {
            rect: interpolate_rect(self.from, self.to, progress),
            progress,
            done: progress >= 1.0,
        }
    }

    pub fn tick(&self, now: Instant) -> AnimationTick {
        let elapsed = now.saturating_duration_since(self.started_at);
        let raw_t = if self.duration.is_zero() {
            1.0
        } else {
            (elapsed.as_secs_f64() / self.duration.as_secs_f64()).clamp(0.0, 1.0)
        };
        self.sample(raw_t)
    }
}

/// Drop an in-flight X11 animation without applying its final target.
pub fn drop_x11_animation(x11_runtime: &mut X11RuntimeConfig, win: WindowId) {
    let _ = x11_runtime.take_window_animation(win);
}

/// Take an in-flight animation and return its current visual rectangle
/// without snapping to the obsolete target — the correct starting point
/// when a live interaction retargets a moving window.
///
/// The X11 arm ticks to `now`; the Wayland arm returns the last frame the
/// compositor rendered (`displayed_frame`), since that is what is actually
/// on screen. The `now` parameter is therefore live for X11 and unused on
/// the Wayland path.
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

/// Slide a newly managed window down into its arranged position.
///
/// The caller must run the window's authoritative arrange first. This helper
/// owns presentation only: it does not initiate layout or mutate window
/// policy. Fullscreen windows skip the decorative transition.
pub(crate) fn run_spawn_animation(ctx: &mut WmCtx, window: WindowId) {
    let animated = ctx.core().behavior().animated;

    let Some((target, is_tiling, is_visible)) =
        ctx.core().model().client_view(window).and_then(|view| {
            if view.client.mode().is_fullscreen() {
                return None;
            }
            Some((
                view.client.geo,
                view.monitor.is_tiling_layout(),
                view.client.is_visible(view.monitor.visible_tags()),
            ))
        })
    else {
        return;
    };

    // On Wayland, newly spawned windows are intentionally left unmapped by
    // apply_visibility until their layout rect has been computed.  Snap them
    // to the authoritative target now so the client receives the layout-size
    // configure and is mapped at the correct rect *before* the decorative
    // spawn transition starts.  This avoids a flash of the client's initial
    // (pre-layout) buffer size.  Invisible windows (hidden tags) are skipped;
    // set_window_target_rect already no-ops unmapped surfaces.  For
    // non-animated mode this also covers the edge case where arrange's
    // move_resize short-circuited (from == target) without mapping.
    if is_visible && let WmCtx::Wayland(_) = ctx {
        ctx.window_backend().resize_window(window, target);
        ctx.window_backend().flush();
    }

    if !animated {
        return;
    }

    ctx.move_resize(
        window,
        target,
        MoveResizeOptions::animate_from(spawn_animation_start(target), DEFAULT_ANIMATION_MILLIS),
    );

    if !is_tiling {
        ctx.window_backend().raise_window_visual_only(window);
        ctx.window_backend().flush();
    }
}

fn spawn_animation_start(target: Rect) -> Rect {
    Rect {
        x: target.x,
        y: target.y - SPAWN_SLIDE_DISTANCE,
        w: target.w,
        h: target.h,
    }
}

fn tag_slide_start(target: Rect, dir: HorizontalDirection) -> Rect {
    let x = match dir {
        HorizontalDirection::Left => target.x - TAG_SLIDE_DISTANCE,
        HorizontalDirection::Right => target.x + TAG_SLIDE_DISTANCE,
    };
    Rect { x, ..target }
}

#[cfg(test)]
mod spawn_animation_tests {
    use super::*;

    #[test]
    fn every_spawn_moves_the_same_fixed_distance() {
        let target = Rect::new(-1200, 500, 700, 400);
        let lower_target = Rect::new(-1200, 900, 700, 400);

        assert_eq!(
            spawn_animation_start(target),
            Rect::new(-1200, 430, 700, 400)
        );
        assert_eq!(
            target.y - spawn_animation_start(target).y,
            SPAWN_SLIDE_DISTANCE
        );
        assert_eq!(
            lower_target.y - spawn_animation_start(lower_target).y,
            SPAWN_SLIDE_DISTANCE
        );
    }
}

pub fn scroll_view_with_slide(ctx: &mut WmCtx, dir: HorizontalDirection) {
    let old_selected_tags = ctx.core().model().expect_selected_monitor().selected_tags();
    let Some(selmon_id) = crate::tags::view::scroll_view_for_slide(ctx, dir) else {
        return;
    };

    crate::layouts::arrange(ctx, Some(selmon_id));

    let (selected_tags, clients) = {
        let Some(monitor) = ctx.core().model().monitor(selmon_id) else {
            return;
        };
        (monitor.selected_tags(), monitor.clients.clone())
    };

    let mut animation_targets = Vec::new();
    for win in clients {
        let Some(client) = ctx.core().model().client(win).cloned() else {
            continue;
        };
        if !client.is_visible(selected_tags)
            || client.is_visible(old_selected_tags)
            || client.mode().is_true_fullscreen()
            || !client.geo.is_valid()
        {
            continue;
        }
        animation_targets.push((win, client.geo));
    }

    for (win, target) in animation_targets {
        ctx.move_resize(
            win,
            target,
            MoveResizeOptions::animate_from(tag_slide_start(target, dir), DEFAULT_ANIMATION_MILLIS),
        );
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

#[cfg(test)]
mod tag_slide_tests {
    use super::*;

    #[test]
    fn tag_slide_starts_a_fixed_distance_from_the_arranged_position() {
        let target = Rect::new(-1200, 40, 1200, 760);

        assert_eq!(
            tag_slide_start(target, HorizontalDirection::Left),
            Rect::new(-1200 - TAG_SLIDE_DISTANCE, 40, 1200, 760)
        );
        assert_eq!(
            tag_slide_start(target, HorizontalDirection::Right),
            Rect::new(-1200 + TAG_SLIDE_DISTANCE, 40, 1200, 760)
        );
    }

    #[test]
    fn full_monitor_window_stays_mostly_visible_at_animation_start() {
        let monitor = Rect::new(1920, 0, 1920, 1080);

        for dir in [HorizontalDirection::Left, HorizontalDirection::Right] {
            let start = tag_slide_start(monitor, dir);
            let visible = start.intersection(&monitor).unwrap();
            assert_eq!(visible.w, monitor.w - TAG_SLIDE_DISTANCE);
            assert_eq!(visible.h, monitor.h);
        }
    }
}
