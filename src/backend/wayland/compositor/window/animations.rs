use smithay::utils::{Logical, Point, Size};
use std::time::{Duration, Instant};

use crate::animation::ease_out_cubic;
use crate::backend::wayland::compositor::WaylandState;
use crate::constants::animation::WAYLAND_DEFAULT_ANIMATION_MILLIS;
use crate::types::{Rect, WindowId};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum WindowMoveMode {
    AnimateTo,
    Immediate,
    AnimateFrom { from: Rect, duration: Duration },
}

/// Send the single expensive client resize around the spatial midpoint of the
/// ease-out transition. `ease_out_cubic(0.2)` is approximately `0.49`; using
/// linear progress `0.5` would wait until 87.5% of the visual motion was over.
/// The frame and cheap compositor motion continue for the full duration.
const RESIZE_CONFIGURE_PHASE: f64 = 0.2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResizeConfigure {
    Unchanged,
    Pending(Size<i32, Logical>),
    Sent(Size<i32, Logical>),
}

impl ResizeConfigure {
    fn toward(committed_size: Size<i32, Logical>, to: Rect) -> Self {
        if committed_size.w == to.w && committed_size.h == to.h {
            Self::Unchanged
        } else {
            Self::Pending(Size::from((to.w.max(1), to.h.max(1))))
        }
    }

    fn advance(&mut self, progress: f64) -> Option<Size<i32, Logical>> {
        let Self::Pending(size) = *self else {
            return None;
        };
        if progress < RESIZE_CONFIGURE_PHASE {
            return None;
        }
        *self = Self::Sent(size);
        Some(size)
    }

    fn target(self) -> Option<Size<i32, Logical>> {
        match self {
            Self::Unchanged => None,
            Self::Pending(size) | Self::Sent(size) => Some(size),
        }
    }
}

/// Wayland presentation state for one logical geometry transition.
///
/// The intended frame interpolates every edge. The client surface keeps its
/// currently committed size and is centered inside that frame, so one-sided
/// growth/shrink creates cheap motion on both sides of the single real resize.
/// Only `ResizeConfigure::Pending` can emit a configure, making repeated
/// client relayout during an animation unrepresentable.
///
/// The border width is part of the transition: it animates from the width the
/// window was displayed with (`from_border`) to the width the requested
/// placement needs (`to_border`). Positioning and rendering both use the
/// current interpolated width, so the animated frame stays faithful to the
/// screen at every instant instead of snapping to the post-transition width
/// on the first frame.
#[derive(Clone, Debug)]
pub(crate) struct WaylandWindowAnimation {
    frame: crate::animation::WindowAnimation,
    displayed_frame: Rect,
    displayed_border: i32,
    from_border: i32,
    to_border: i32,
    resize: ResizeConfigure,
}

#[derive(Clone, Copy, Debug)]
struct WaylandAnimationTick {
    previous_frame: Rect,
    frame: Rect,
    surface_location: Point<i32, Logical>,
    configure_size: Option<Size<i32, Logical>>,
    done: bool,
}

/// Blend a border width across eased animation progress so the border keeps
/// pace with the frame motion.
fn interpolate_borders(from: i32, to: i32, eased: f64) -> i32 {
    (from as f64 + (to as f64 - from as f64) * eased).round() as i32
}

impl WaylandWindowAnimation {
    fn new(
        from: Rect,
        to: Rect,
        committed_size: Size<i32, Logical>,
        duration: Duration,
        now: Instant,
        from_border: i32,
        to_border: i32,
    ) -> Self {
        Self {
            frame: crate::animation::WindowAnimation {
                from,
                to,
                started_at: now,
                duration,
            },
            displayed_frame: from,
            displayed_border: from_border,
            from_border,
            to_border,
            resize: ResizeConfigure::toward(committed_size, to),
        }
    }

    fn target(&self) -> Rect {
        self.frame.to
    }

    fn displayed_frame(&self) -> Rect {
        self.displayed_frame
    }

    /// The border width currently presented for this window: the interpolated
    /// value at the latest frame, `from_border` before the first tick and
    /// `to_border` once the transition is complete.
    fn displayed_border_width(&self) -> i32 {
        self.displayed_border
    }

    /// The border width the final placement requests.
    fn target_border_width(&self) -> i32 {
        self.to_border
    }

    fn resize_target(&self) -> Option<Size<i32, Logical>> {
        self.resize.target()
    }

    fn tick(&mut self, now: Instant, committed_size: Size<i32, Logical>) -> WaylandAnimationTick {
        let previous_frame = self.displayed_frame;
        let tick = self.frame.tick(now);
        let eased = ease_out_cubic(tick.progress);
        self.displayed_frame = tick.rect;
        self.displayed_border = interpolate_borders(self.from_border, self.to_border, eased);
        WaylandAnimationTick {
            previous_frame,
            frame: tick.rect,
            surface_location: centered_surface_location(
                tick.rect,
                committed_size,
                self.displayed_border,
            ),
            configure_size: self.resize.advance(tick.progress),
            done: tick.done,
        }
    }
}

fn centered_surface_location(
    frame: Rect,
    committed_size: Size<i32, Logical>,
    border_width: i32,
) -> Point<i32, Logical> {
    let border_width = border_width.max(0);
    Point::from((
        frame.x + border_width + (frame.w - committed_size.w) / 2,
        frame.y + border_width + (frame.h - committed_size.h) / 2,
    ))
}

impl WaylandState {
    pub(crate) fn set_layout_preview_target(
        &mut self,
        target: Option<Rect>,
        animate: bool,
        duration: Duration,
    ) {
        self.layout_preview_animation
            .set_target(target, animate, duration, Instant::now());
        self.request_render();
    }

    pub(crate) fn layout_preview_rect(&self) -> Option<Rect> {
        self.layout_preview_animation.displayed()
    }

    pub(crate) fn has_active_layout_preview_animation(&self) -> bool {
        self.layout_preview_animation.is_active()
    }

    fn insert_or_replace_window_animation(
        &mut self,
        window_id: WindowId,
        animation: WaylandWindowAnimation,
    ) {
        self.window_animations.insert(window_id, animation);
    }

    pub(crate) fn drop_window_animation(&mut self, win: WindowId) {
        self.window_animations.remove(&win);
    }

    pub(crate) fn animations_enabled(&self) -> bool {
        self.globals()
            .map(|state| state.behavior.animated)
            .unwrap_or(false)
    }

    fn configured_animation_duration(&self, duration: Duration) -> Duration {
        self.globals()
            .map(|core| core.config.animations.scale_duration(duration))
            .unwrap_or(duration)
    }

    pub(crate) fn interactive_motion_active(&self) -> bool {
        self.globals()
            .is_some_and(|state| state.drag.active_interaction().is_some())
    }

    pub(crate) fn default_window_move_mode(&self) -> WindowMoveMode {
        if self.interactive_motion_active() {
            WindowMoveMode::Immediate
        } else {
            WindowMoveMode::AnimateTo
        }
    }

    fn configured_size_unchanged(&self, window_id: WindowId, target: Rect) -> bool {
        let configured_size = (target.w.max(1), target.h.max(1));
        self.last_configured_size
            .get(&window_id)
            .is_some_and(|&size| size == configured_size)
    }

    fn remap_window_immediately(
        &mut self,
        window_id: WindowId,
        element: &smithay::desktop::Window,
        target_loc: Point<i32, Logical>,
    ) {
        self.drop_window_animation(window_id);
        self.remap_element_preserving_z_order(element, target_loc, false);
    }

    fn configure_window_geometry_if_needed(
        &mut self,
        window_id: WindowId,
        element: &smithay::desktop::Window,
        target: Rect,
    ) {
        let configured = (target.w.max(1), target.h.max(1));
        if self
            .last_configured_size
            .get(&window_id)
            .is_some_and(|&size| size == configured)
        {
            return;
        }

        if element.toplevel().is_some() {
            self.send_toplevel_configure(element, Some(Size::from(configured)));
        } else if let Some(surface) = element.x11_surface() {
            let geometry =
                smithay::utils::Rectangle::new((target.x, target.y).into(), configured.into());
            let _ = surface.configure(Some(geometry));
        }
        self.last_configured_size.insert(window_id, configured);
    }

    /// Place a window at `target` (in outer/WM coordinates) using the given
    /// movement mode.
    ///
    /// This is a **visual placement** function. It converts the outer WM rect
    /// to inner/surface coordinates (adding `border_width`) and either snaps
    /// or animates the element to the target. Animated resizes send exactly
    /// one configure partway through the transition; snapped changes send it
    /// immediately.
    ///
    /// It does **not** write to `client.geo`.  The WM layer owns logical
    /// position and always sets `client.geo` before calling this function
    /// (or via `sync_space_from_globals`).
    pub(crate) fn set_window_target_rect(
        &mut self,
        window_id: WindowId,
        target: Rect,
        mode: WindowMoveMode,
    ) {
        let Some(element) = self.find_window(window_id).cloned() else {
            return;
        };
        let Some(to_border) = self
            .globals()
            .and_then(|state| state.model.client(window_id).map(|c| c.border_width))
        else {
            return;
        };

        // Convert outer WM rect → inner surface rect.
        let target_loc: Point<i32, Logical> =
            Point::from((target.x + to_border, target.y + to_border));
        let actual_loc = self.space.element_location(&element);

        // Keep an in-flight animation when callers repeatedly request the
        // same target (e.g. sync_space_from_globals during a decorative
        // AnimateFrom slide-in, or an immediate move that preserves an
        // animation already landing on this rect).
        if self
            .window_animations
            .get(&window_id)
            .is_some_and(|anim| anim.target() == target)
        {
            return;
        }

        // Geometry updates for hidden/unmapped windows must not remap them.
        // The WM layer owns visibility.
        if actual_loc.is_none() && mode != WindowMoveMode::Immediate {
            self.drop_window_animation(window_id);
            return;
        }

        // Skip if already at the target with unchanged size.
        let size_unchanged = self.configured_size_unchanged(window_id, target);
        if actual_loc == Some(target_loc) && mode == WindowMoveMode::AnimateTo && size_unchanged {
            self.drop_window_animation(window_id);
            return;
        }

        // The border width the window is actually presented with right now.
        //
        // During an in-flight transition the window follows the animation's own
        // interpolated border. After a placement the recorded width stays with
        // the model's pre-transition value, so a subsequent transition starts
        // from the border the window was visibly drawn with, not the already
        // mutated post-transition value. Unplaced windows fall back to the
        // requested target width.
        let from_border = self
            .window_animations
            .get(&window_id)
            .map(WaylandWindowAnimation::displayed_border_width)
            .or_else(|| self.placed_border.get(&window_id).copied())
            .unwrap_or(to_border);

        // Resolve the displayed outer frame at animation start. The actual
        // committed surface size deliberately remains independent.
        let actual_size = element.geometry().size;
        let (from, animation_duration) = match mode {
            WindowMoveMode::AnimateFrom { from, duration } => (from, duration),
            WindowMoveMode::AnimateTo | WindowMoveMode::Immediate => {
                let loc = actual_loc.unwrap_or(target_loc);
                (
                    Rect {
                        x: loc.x - from_border,
                        y: loc.y - from_border,
                        w: actual_size.w.max(1),
                        h: actual_size.h.max(1),
                    },
                    self.configured_animation_duration(Duration::from_millis(
                        WAYLAND_DEFAULT_ANIMATION_MILLIS,
                    )),
                )
            }
        };

        let should_snap =
            !self.animations_enabled() || mode == WindowMoveMode::Immediate || from == target;

        if should_snap {
            self.configure_window_geometry_if_needed(window_id, &element, target);
            self.remap_window_immediately(window_id, &element, target_loc);
            self.placed_border.insert(window_id, to_border);
            return;
        }

        let start_loc = centered_surface_location(from, actual_size, from_border);
        if actual_loc != Some(start_loc) {
            self.remap_element_preserving_z_order(&element, start_loc, false);
        }
        self.placed_border.insert(window_id, from_border);

        self.insert_or_replace_window_animation(
            window_id,
            WaylandWindowAnimation::new(
                from,
                target,
                actual_size,
                animation_duration,
                Instant::now(),
                from_border,
                to_border,
            ),
        );
    }

    /// Cancel a single window's in-flight animation.
    ///
    /// If the window is currently mapped (has a location in the space), it is
    /// snapped to the animation's target position. If not mapped, the animation
    /// entry is simply dropped without remapping.
    pub fn cancel_window_animation(&mut self, win: WindowId) {
        let Some(anim) = self.window_animations.remove(&win) else {
            return;
        };
        if let Some(element) = self.find_window(win).cloned()
            && self.space.element_location(&element).is_some()
        {
            let target = anim.target();
            let border_width = anim.target_border_width();
            self.request_visual_rect_render(anim.displayed_frame().with_borders(border_width));
            self.request_visible_window_render(&element);
            if anim.resize_target().is_some() {
                // A cancellation before the staging point still has to
                // deliver the final resize. A resize already sent during the
                // animation is suppressed by the configured-size cache.
                self.configure_window_geometry_if_needed(win, &element, target);
            }
            let loc = Point::from((target.x + border_width, target.y + border_width));
            self.remap_element_preserving_z_order(&element, loc, false);
            self.placed_border.insert(win, border_width);
            self.request_visible_window_render(&element);
            self.request_visual_rect_render(target.with_borders(border_width));
        }
    }

    /// Remove an in-flight animation and return its currently displayed frame
    /// in the WM's outer-coordinate convention.
    pub(crate) fn take_current_window_animation_rect(
        &mut self,
        win: WindowId,
        now: Instant,
    ) -> Option<Rect> {
        let animation = self.window_animations.remove(&win)?;
        Some(animation.frame.tick(now).rect)
    }

    /// Tick all active window animations.
    pub fn tick_animations(&mut self) {
        let preview_active = self.layout_preview_animation.is_active();
        if self.window_animations.is_empty() && !preview_active {
            return;
        }
        let now = Instant::now();
        if preview_active {
            self.layout_preview_animation.tick(now);
            self.request_render();
        }
        let animation_inputs: Vec<_> = self
            .window_animations
            .keys()
            .map(|&win| {
                (
                    win,
                    self.find_window(win).map(|element| element.geometry().size),
                )
            })
            .collect();
        let mut updates = Vec::with_capacity(animation_inputs.len());
        let mut finished: Vec<WindowId> = Vec::new();
        for (win, committed_size) in animation_inputs {
            let Some(committed_size) = committed_size else {
                finished.push(win);
                continue;
            };
            if let Some(animation) = self.window_animations.get_mut(&win) {
                let tick = animation.tick(now, committed_size);
                let border_width = animation.displayed_border_width();
                let configure_target = tick.configure_size.map(|_| animation.target());
                updates.push((
                    win,
                    tick.surface_location,
                    configure_target,
                    tick.previous_frame,
                    tick.frame,
                    border_width,
                    tick.done,
                ));
            }
        }

        for (win, loc, configure_target, previous_frame, frame, border_width, done) in updates {
            if let Some(element) = self.find_window(win).cloned() {
                self.request_visual_rect_render(previous_frame.with_borders(border_width));
                self.request_visible_window_render(&element);
                if let Some(target) = configure_target {
                    self.configure_window_geometry_if_needed(win, &element, target);
                }
                if self.space.element_location(&element) != Some(loc) {
                    self.remap_element_preserving_z_order(&element, loc, false);
                }
                self.placed_border.insert(win, border_width);
                // Preserve damage on both sides of a cross-output move and
                // redraw animated borders even when a symmetric resize keeps
                // the surface center stationary.
                self.request_visible_window_render(&element);
                self.request_visual_rect_render(frame.with_borders(border_width));
            } else {
                finished.push(win);
                continue;
            }
            if done {
                finished.push(win);
            }
        }
        for win in finished {
            self.cancel_window_animation(win);
        }
    }

    /// Cancel all in-flight window animations, snapping each mapped window
    /// to its animation target position.
    pub fn cancel_all_window_animations(&mut self) {
        let active_windows: Vec<WindowId> = self.window_animations.keys().copied().collect();
        for win in active_windows {
            self.cancel_window_animation(win);
        }
    }

    /// Check if there are active window animations.
    pub fn has_active_window_animations(&self) -> bool {
        !self.window_animations.is_empty()
    }

    /// Check if any compositor visual transition needs another frame.
    pub fn has_active_animations(&self) -> bool {
        self.has_active_window_animations() || self.layout_preview_animation.is_active()
    }

    /// Check if the window has an in-flight animation heading toward `outer_target`
    /// (in WM/outer coordinates).  The border-width conversion is handled
    /// internally so callers don't need to know about the inner coordinate space.
    pub(crate) fn animation_targets_outer_rect(&self, win: WindowId, outer_target: Rect) -> bool {
        self.window_animations
            .get(&win)
            .is_some_and(|animation| animation.target() == outer_target)
    }

    pub(crate) fn displayed_animation_frame(&self, win: WindowId) -> Option<Rect> {
        self.window_animations
            .get(&win)
            .map(WaylandWindowAnimation::displayed_frame)
    }

    /// The border width the window is currently *presented* with.
    ///
    /// While animated the interpolated transition border wins; otherwise the
    /// authoritative model width applies (`fallback`). Render and hit-test
    /// code use this instead of reading `client.border_width` directly so they
    /// agree with the displayed frame during transitions.
    pub(crate) fn presented_border_width(&self, win: WindowId, fallback: i32) -> i32 {
        self.window_animations
            .get(&win)
            .map(WaylandWindowAnimation::displayed_border_width)
            .unwrap_or(fallback)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_schedule_emits_exactly_one_configure() {
        let start = Instant::now();
        let mut animation = WaylandWindowAnimation::new(
            Rect::new(0, 0, 100, 80),
            Rect::new(0, 0, 140, 60),
            Size::from((100, 80)),
            Duration::from_millis(100),
            start,
            2,
            4,
        );

        assert_eq!(
            animation
                .tick(start + Duration::from_millis(19), Size::from((100, 80)))
                .configure_size,
            None
        );
        assert_eq!(
            animation
                .tick(start + Duration::from_millis(21), Size::from((100, 80)))
                .configure_size,
            Some(Size::from((140, 60)))
        );
        assert_eq!(
            animation
                .tick(start + Duration::from_millis(80), Size::from((140, 60)))
                .configure_size,
            None
        );
        assert!(matches!(animation.resize, ResizeConfigure::Sent(_)));
    }

    #[test]
    fn movement_only_transition_never_schedules_a_resize() {
        let start = Instant::now();
        let mut animation = WaylandWindowAnimation::new(
            Rect::new(0, 0, 100, 80),
            Rect::new(50, 20, 100, 80),
            Size::from((100, 80)),
            Duration::from_millis(100),
            start,
            0,
            0,
        );

        assert_eq!(
            animation
                .tick(start + Duration::from_millis(100), Size::from((100, 80)))
                .configure_size,
            None
        );
        assert_eq!(animation.resize, ResizeConfigure::Unchanged);
    }

    #[test]
    fn resize_schedule_compares_target_with_committed_not_visual_size() {
        let start = Instant::now();
        let needs_resize = WaylandWindowAnimation::new(
            Rect::new(0, 0, 120, 80),
            Rect::new(10, 0, 120, 80),
            Size::from((140, 80)),
            Duration::from_millis(100),
            start,
            0,
            0,
        );
        let visual_size_differs_but_client_is_ready = WaylandWindowAnimation::new(
            Rect::new(0, 0, 120, 80),
            Rect::new(10, 0, 140, 80),
            Size::from((140, 80)),
            Duration::from_millis(100),
            start,
            0,
            0,
        );

        assert!(matches!(needs_resize.resize, ResizeConfigure::Pending(_)));
        assert_eq!(
            visual_size_differs_but_client_is_ready.resize,
            ResizeConfigure::Unchanged
        );
    }

    #[test]
    fn borders_interpolate_from_the_displayed_to_the_target_width() {
        let start = Instant::now();
        let mut animation = WaylandWindowAnimation::new(
            Rect::new(0, 30, 1200, 740),
            Rect::new(150, 126, 896, 573),
            Size::from((1200, 740)),
            Duration::from_millis(100),
            start,
            0,
            2,
        );

        // Starts at the displayed (pre-transition) width: a borderless tile.
        assert_eq!(animation.displayed_border_width(), 0);

        // Early in the eased transition the width has not reached the target.
        animation.tick(start + Duration::from_millis(10), Size::from((1200, 740)));
        let early = animation.displayed_border_width();
        assert!(early > 0 && early < 2);

        // Border and buffer-size presentation are independent: at the visual
        // midpoint, the old committed buffer remains centered in the frame
        // while the border follows the same eased progress as the frame.
        let midpoint = animation.tick(start + Duration::from_millis(50), Size::from((1200, 740)));
        assert_eq!(midpoint.surface_location, Point::from((0, 43)));
        assert_eq!(animation.displayed_border_width(), 2);

        // The transition ends fully bordered, landed exactly on the target
        // inner rectangle: content origin offset by the final border width.
        let tick = animation.tick(start + Duration::from_millis(100), Size::from((896, 573)));
        assert_eq!(tick.surface_location, Point::from((152, 128)));
        assert_eq!(animation.displayed_border_width(), 2);
        assert_eq!(animation.target_border_width(), 2);
    }

    #[test]
    fn one_sided_growth_moves_before_and_after_the_size_switch() {
        let halfway_frame = Rect::new(0, 0, 120, 80);

        let before_resize = centered_surface_location(halfway_frame, Size::from((100, 80)), 0);
        let after_resize = centered_surface_location(halfway_frame, Size::from((140, 80)), 0);

        assert_eq!(before_resize, Point::from((10, 0)));
        assert_eq!(after_resize, Point::from((-10, 0)));
        assert_eq!(
            centered_surface_location(Rect::new(0, 0, 140, 80), Size::from((140, 80)), 0,),
            Point::from((0, 0))
        );
    }

    #[test]
    fn one_sided_shrink_moves_before_and_after_the_size_switch() {
        let halfway_frame = Rect::new(0, 0, 80, 80);

        let before_resize = centered_surface_location(halfway_frame, Size::from((100, 80)), 0);
        let after_resize = centered_surface_location(halfway_frame, Size::from((60, 80)), 0);

        assert_eq!(before_resize, Point::from((-10, 0)));
        assert_eq!(after_resize, Point::from((10, 0)));
        assert_eq!(
            centered_surface_location(Rect::new(0, 0, 60, 80), Size::from((60, 80)), 0,),
            Point::from((0, 0))
        );
    }

    #[test]
    fn every_edge_combination_keeps_the_surface_centered_in_the_visual_frame() {
        let border = 4;

        for left_delta in [-20, 0, 20] {
            for right_delta in [-20, 0, 20] {
                for top_delta in [-20, 0, 20] {
                    for bottom_delta in [-20, 0, 20] {
                        let left = left_delta / 2;
                        let right = 100 + right_delta / 2;
                        let top = top_delta / 2;
                        let bottom = 80 + bottom_delta / 2;
                        let frame = Rect::new(left, top, right - left, bottom - top);
                        assert!(frame.is_valid());

                        let committed_sizes = [
                            Size::from((100, 80)),
                            Size::from((frame.w, frame.h)),
                            Size::from((120, 60)),
                        ];

                        for committed in committed_sizes {
                            let location = centered_surface_location(frame, committed, border);
                            assert_eq!(
                                2 * location.x + committed.w,
                                2 * (frame.x + border) + frame.w
                            );
                            assert_eq!(
                                2 * location.y + committed.h,
                                2 * (frame.y + border) + frame.h
                            );
                        }
                    }
                }
            }
        }
    }
}
