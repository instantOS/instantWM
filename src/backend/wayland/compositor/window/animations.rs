use smithay::utils::{Logical, Point, Size};
use std::time::{Duration, Instant};

use crate::animation::ease_out_cubic;
use crate::backend::wayland::compositor::WaylandState;
use crate::constants::animation::WAYLAND_DEFAULT_ANIMATION_MILLIS;
use crate::types::{Rect, WindowId};

/// Backend-resolved placement mode for a Wayland window — the Wayland
/// compositor's counterpart to the core `MoveResizeMode`. Unlike the core
/// enum (caller intent), these variants carry *resolved* semantics:
/// [`Retarget`](Self::Retarget) is the only mode that lets the backend resolve
/// the origin itself, by peeking an in-flight animation so a live transition
/// can be retargeted without restarting. Durations are always supplied by the
/// caller; the backend never synthesizes a hidden default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum WindowMoveMode {
    /// Animate from the surface's current visual position to `target`. The
    /// backend peeks any in-flight animation to keep the transition continuous
    /// and preserves it when the target is unchanged. `duration` is supplied
    /// by the caller (e.g. the default-move resolver).
    Retarget { duration: Duration },
    /// Place at `target` immediately, without a transition.
    Snap,
    /// Animate from an explicit `from` rect to `target` over `duration`. The
    /// caller has already resolved both the origin and the (scaled) duration.
    AnimateFrom { from: Rect, duration: Duration },
}

/// Normally send the single expensive client resize around the spatial
/// midpoint of the ease-out transition. `ease_out_cubic(0.2)` is approximately
/// `0.49`; using linear progress `0.5` would wait until 87.5% of the visual
/// motion was over. Fully offscreen growth is configured before movement;
/// mostly offscreen removal is configured while the final movement can still
/// mask client relayout.
const OFFSCREEN_GROWTH_CONFIGURE_PHASE: f64 = 0.0;
const RESIZE_CONFIGURE_PHASE: f64 = 0.2;
const MAX_VISIBLE_OFFSCREEN_RESIZE_PERCENT: i64 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResizeTiming {
    Normal,
    OffscreenGrowth,
    OffscreenShrink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResizeConfigure {
    Unchanged,
    Pending(Size<i32, Logical>),
    Sent(Size<i32, Logical>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SurfaceAnchor {
    Near,
    Far,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SurfaceAnchors {
    x: SurfaceAnchor,
    y: SurfaceAnchor,
}

impl SurfaceAnchors {
    fn between(from: Rect, to: Rect, from_border: i32, to_border: i32) -> Self {
        Self {
            x: axis_anchor(from.x, from.w, from_border, to.x, to.w, to_border),
            y: axis_anchor(from.y, from.h, from_border, to.y, to.h, to_border),
        }
    }

    /// Retargeting samples a new visual frame while the client may still be
    /// displaying an older committed size. Keep the anchor whose placement is
    /// closest to the current surface location; changing policy here would
    /// itself create a teleport.
    fn continuous_with(
        self,
        frame: Rect,
        committed_size: Size<i32, Logical>,
        border_width: i32,
        current_location: Point<i32, Logical>,
    ) -> Self {
        Self {
            x: closest_axis_anchor(
                frame.x,
                frame.w,
                committed_size.w,
                border_width,
                current_location.x,
                self.x,
            ),
            y: closest_axis_anchor(
                frame.y,
                frame.h,
                committed_size.h,
                border_width,
                current_location.y,
                self.y,
            ),
        }
    }
}

/// Follow the edge that travels farther so compositor-side translation masks
/// the instantaneous client resize. Equal travel uses the near edge, keeping
/// symmetric expansion visibly moving toward the top-left.
fn axis_anchor(
    from_position: i32,
    from_size: i32,
    from_border: i32,
    to_position: i32,
    to_size: i32,
    to_border: i32,
) -> SurfaceAnchor {
    let presented_edges = |position: i32, size: i32, border: i32| {
        let near = i64::from(position) + i64::from(border.max(0));
        (near, near + i64::from(size))
    };
    let (from_near, from_far) = presented_edges(from_position, from_size, from_border);
    let (to_near, to_far) = presented_edges(to_position, to_size, to_border);
    let near_travel = (to_near - from_near).abs();
    let far_travel = (to_far - from_far).abs();

    if near_travel >= far_travel {
        SurfaceAnchor::Near
    } else {
        SurfaceAnchor::Far
    }
}

fn closest_axis_anchor(
    frame_position: i32,
    frame_size: i32,
    committed_size: i32,
    border_width: i32,
    current_position: i32,
    preferred: SurfaceAnchor,
) -> SurfaceAnchor {
    let near = frame_position + border_width.max(0);
    let far = near + frame_size - committed_size;
    let distance = |candidate: i32| (i64::from(current_position) - i64::from(candidate)).abs();

    match distance(near).cmp(&distance(far)) {
        std::cmp::Ordering::Less => SurfaceAnchor::Near,
        std::cmp::Ordering::Greater => SurfaceAnchor::Far,
        std::cmp::Ordering::Equal => preferred,
    }
}

impl ResizeConfigure {
    fn toward(
        committed_size: Size<i32, Logical>,
        last_configured_size: Option<(i32, i32)>,
        to: Rect,
    ) -> Self {
        let target = (to.w.max(1), to.h.max(1));
        let committed_matches = committed_size.w == target.0 && committed_size.h == target.1;
        let stale_configure_outstanding = last_configured_size.is_some_and(|size| size != target);
        if committed_matches && !stale_configure_outstanding {
            Self::Unchanged
        } else {
            Self::Pending(Size::from(target))
        }
    }

    fn advance(&mut self, progress: f64, configure_phase: f64) -> Option<Size<i32, Logical>> {
        let Self::Pending(size) = *self else {
            return None;
        };
        if progress < configure_phase {
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
/// currently committed size and follows the frame edge that travels farther
/// on each axis. The opposite edge absorbs the single real resize while the
/// surface is visibly moving. Only `ResizeConfigure::Pending` can
/// emit a configure, making repeated client relayout during an animation
/// unrepresentable.
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
    anchors: SurfaceAnchors,
    resize: ResizeConfigure,
    resize_timing: ResizeTiming,
    resize_configure_phase: f64,
    shrink_stage_presented: bool,
    waiting_for_resize: bool,
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
        last_configured_size: Option<(i32, i32)>,
        duration: Duration,
        now: Instant,
        from_border: i32,
        to_border: i32,
    ) -> Self {
        let anchors = SurfaceAnchors::between(from, to, from_border, to_border);
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
            anchors,
            resize: ResizeConfigure::toward(committed_size, last_configured_size, to),
            resize_timing: ResizeTiming::Normal,
            resize_configure_phase: RESIZE_CONFIGURE_PHASE,
            shrink_stage_presented: false,
            waiting_for_resize: false,
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

    fn needs_landing(&self, committed_size: Size<i32, Logical>) -> bool {
        let target = self.target();
        (self.anchors.x == SurfaceAnchor::Far && committed_size.w != target.w)
            || (self.anchors.y == SurfaceAnchor::Far && committed_size.h != target.h)
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.waiting_for_resize
    }

    fn revalidate_offscreen_resize_phase(
        &mut self,
        committed_size: Size<i32, Logical>,
        outputs: &[Rect],
    ) {
        match self.resize_timing {
            ResizeTiming::Normal => {}
            ResizeTiming::OffscreenGrowth => {
                if !resize_growth_is_offscreen_at_start(
                    self.frame.from,
                    self.target(),
                    committed_size,
                    self.from_border,
                    self.anchors,
                    outputs,
                ) {
                    self.resize_timing = ResizeTiming::Normal;
                    self.resize_configure_phase = RESIZE_CONFIGURE_PHASE;
                }
            }
            ResizeTiming::OffscreenShrink => {
                if let Some(phase) = offscreen_shrink_configure_phase(
                    self.frame.from,
                    self.target(),
                    committed_size,
                    self.from_border,
                    self.to_border,
                    self.anchors,
                    outputs,
                ) {
                    self.resize_configure_phase = phase;
                } else {
                    self.resize_timing = ResizeTiming::Normal;
                    self.resize_configure_phase = RESIZE_CONFIGURE_PHASE;
                }
            }
        }
    }

    fn tick(&mut self, now: Instant, committed_size: Size<i32, Logical>) -> WaylandAnimationTick {
        let previous_frame = self.displayed_frame;
        let mut tick = self.frame.tick(now);
        let should_present_shrink_stage = self.resize_timing == ResizeTiming::OffscreenShrink
            && !self.shrink_stage_presented
            && matches!(self.resize, ResizeConfigure::Pending(_))
            && tick.progress >= self.resize_configure_phase;
        if should_present_shrink_stage {
            tick.progress = self.resize_configure_phase;
            tick.rect = interpolated_rect(self.frame.from, self.frame.to, tick.progress);
            tick.done = false;
            self.shrink_stage_presented = true;
        }
        let eased = ease_out_cubic(tick.progress);
        self.displayed_frame = tick.rect;
        self.displayed_border = interpolate_borders(self.from_border, self.to_border, eased);
        WaylandAnimationTick {
            previous_frame,
            frame: tick.rect,
            surface_location: anchored_surface_location(
                tick.rect,
                committed_size,
                self.displayed_border,
                self.anchors,
            ),
            configure_size: self
                .resize
                .advance(tick.progress, self.resize_configure_phase),
            done: tick.done,
        }
    }
}

fn anchored_surface_location(
    frame: Rect,
    committed_size: Size<i32, Logical>,
    border_width: i32,
    anchors: SurfaceAnchors,
) -> Point<i32, Logical> {
    let border_width = border_width.max(0);
    let offset = |frame_size: i32, committed_size: i32, anchor: SurfaceAnchor| match anchor {
        SurfaceAnchor::Near => 0,
        SurfaceAnchor::Far => frame_size - committed_size,
    };
    Point::from((
        frame.x + border_width + offset(frame.w, committed_size.w, anchors.x),
        frame.y + border_width + offset(frame.h, committed_size.h, anchors.y),
    ))
}

fn rectangle_union_area(rects: &[Rect]) -> i64 {
    let mut x_edges: Vec<_> = rects
        .iter()
        .flat_map(|rect| [rect.x, rect.right()])
        .collect();
    x_edges.sort_unstable();
    x_edges.dedup();

    x_edges
        .windows(2)
        .map(|x| {
            let mut intervals: Vec<_> = rects
                .iter()
                .filter(|rect| rect.x < x[1] && rect.right() > x[0])
                .map(|rect| (rect.y, rect.bottom()))
                .collect();
            intervals.sort_unstable();

            let mut covered_y = 0_i64;
            let mut current: Option<(i32, i32)> = None;
            for (start, end) in intervals {
                match current {
                    Some((current_start, current_end)) if start <= current_end => {
                        current = Some((current_start, current_end.max(end)));
                    }
                    Some((current_start, current_end)) => {
                        covered_y += i64::from(current_end) - i64::from(current_start);
                        current = Some((start, end));
                    }
                    None => current = Some((start, end)),
                }
            }
            if let Some((start, end)) = current {
                covered_y += i64::from(end) - i64::from(start);
            }
            (i64::from(x[1]) - i64::from(x[0])) * covered_y
        })
        .sum()
}

/// Determine whether at least 95% of the pixels in `larger_size` but not
/// `smaller_size` are outside the union of all outputs when both surfaces
/// follow the same anchored frame.
fn extra_surface_pixels_are_offscreen(
    frame: Rect,
    smaller_size: Size<i32, Logical>,
    larger_size: Size<i32, Logical>,
    border_width: i32,
    anchors: SurfaceAnchors,
    outputs: &[Rect],
) -> bool {
    if outputs.is_empty()
        || smaller_size.w > larger_size.w
        || smaller_size.h > larger_size.h
        || smaller_size == larger_size
    {
        return false;
    }

    let smaller_loc = anchored_surface_location(frame, smaller_size, border_width, anchors);
    let larger_loc = anchored_surface_location(frame, larger_size, border_width, anchors);
    let smaller_rect = Rect::new(smaller_loc.x, smaller_loc.y, smaller_size.w, smaller_size.h);
    let larger_rect = Rect::new(larger_loc.x, larger_loc.y, larger_size.w, larger_size.h);
    let visible_intersections = |surface: Rect| {
        outputs
            .iter()
            .filter_map(|output| surface.intersection(output))
            .collect::<Vec<_>>()
    };
    let total_extra = i64::from(larger_size.w) * i64::from(larger_size.h)
        - i64::from(smaller_size.w) * i64::from(smaller_size.h);
    let visible_larger = rectangle_union_area(&visible_intersections(larger_rect));
    let visible_smaller = rectangle_union_area(&visible_intersections(smaller_rect));
    if visible_smaller > visible_larger {
        return false;
    }
    let visible_extra = visible_larger - visible_smaller;

    i128::from(visible_extra) * 100
        <= i128::from(total_extra) * i128::from(MAX_VISIBLE_OFFSCREEN_RESIZE_PERCENT)
}

#[cfg(test)]
fn resize_removal_is_offscreen_at_target(
    target: Rect,
    committed_size: Size<i32, Logical>,
    border_width: i32,
    anchors: SurfaceAnchors,
    outputs: &[Rect],
) -> bool {
    let target_size = Size::from((target.w.max(1), target.h.max(1)));
    extra_surface_pixels_are_offscreen(
        target,
        target_size,
        committed_size,
        border_width,
        anchors,
        outputs,
    )
}

fn resize_growth_is_offscreen_at_start(
    from: Rect,
    target: Rect,
    committed_size: Size<i32, Logical>,
    border_width: i32,
    anchors: SurfaceAnchors,
    outputs: &[Rect],
) -> bool {
    let target_size = Size::from((target.w.max(1), target.h.max(1)));
    extra_surface_pixels_are_offscreen(
        from,
        committed_size,
        target_size,
        border_width,
        anchors,
        outputs,
    )
}

fn interpolated_rect(from: Rect, to: Rect, linear_progress: f64) -> Rect {
    let eased = ease_out_cubic(linear_progress);
    let interpolate =
        |start: i32, end: i32| (f64::from(start) + f64::from(end - start) * eased).round() as i32;
    Rect::new(
        interpolate(from.x, to.x),
        interpolate(from.y, to.y),
        interpolate(from.w, to.w),
        interpolate(from.h, to.h),
    )
}

/// Find the first sampled point where at least 95% of a shrink's removed area
/// is outside all outputs. Sending there leaves a small amount of compositor
/// motion to hide clients that commit more than one relayout buffer.
fn offscreen_shrink_configure_phase(
    from: Rect,
    target: Rect,
    committed_size: Size<i32, Logical>,
    from_border: i32,
    to_border: i32,
    anchors: SurfaceAnchors,
    outputs: &[Rect],
) -> Option<f64> {
    const PHASE_STEPS: u32 = 100;
    let target_size = Size::from((target.w.max(1), target.h.max(1)));

    // Earlier than the normal phase buys us nothing, and the endpoint leaves
    // no compositor movement to cover the client's relayout.
    (21..PHASE_STEPS).find_map(|step| {
        let phase = f64::from(step) / f64::from(PHASE_STEPS);
        let frame = interpolated_rect(from, target, phase);
        let border = interpolate_borders(from_border, to_border, ease_out_cubic(phase));
        let surface_location = anchored_surface_location(frame, committed_size, border, anchors);
        let target_surface_location =
            anchored_surface_location(target, committed_size, to_border, anchors);
        (surface_location != target_surface_location
            && extra_surface_pixels_are_offscreen(
                frame,
                target_size,
                committed_size,
                border,
                anchors,
                outputs,
            ))
        .then_some(phase)
    })
}

impl WaylandState {
    pub(crate) fn set_layout_preview_target(
        &mut self,
        target: Option<Rect>,
        style: crate::types::InteractionOutlineStyle,
        animate: bool,
        duration: Duration,
    ) {
        self.layout_preview_style = style;
        self.layout_preview_animation
            .set_target(target, animate, duration, Instant::now());
        self.request_render();
    }

    pub(crate) fn layout_preview_rect(&self) -> Option<Rect> {
        self.layout_preview_animation.displayed()
    }

    pub(crate) fn layout_preview_style(&self) -> crate::types::InteractionOutlineStyle {
        self.layout_preview_style
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
            WindowMoveMode::Snap
        } else {
            WindowMoveMode::Retarget {
                duration: self.default_animation_duration(),
            }
        }
    }

    pub(crate) fn default_animation_duration(&self) -> Duration {
        self.configured_animation_duration(Duration::from_millis(WAYLAND_DEFAULT_ANIMATION_MILLIS))
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

    /// Snap `element` to `target`: configure its geometry, remap without
    /// disturbing z-order, and record the border width. Shared by an explicit
    /// [`WindowMoveMode::Snap`] and by an animated mode that resolves to a
    /// no-transition placement (animations disabled, or `from == target`).
    fn snap_window_to(
        &mut self,
        window_id: WindowId,
        element: &smithay::desktop::Window,
        target: Rect,
        target_loc: Point<i32, Logical>,
        to_border: i32,
    ) {
        self.configure_window_geometry_if_needed(window_id, element, target);
        self.remap_window_immediately(window_id, element, target_loc);
        self.placed_border.insert(window_id, to_border);
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
        if actual_loc.is_none() && mode != WindowMoveMode::Snap {
            self.drop_window_animation(window_id);
            return;
        }

        // Skip if already at the target with unchanged size.
        let size_unchanged = self.configured_size_unchanged(window_id, target);
        if actual_loc == Some(target_loc)
            && matches!(mode, WindowMoveMode::Retarget { .. })
            && size_unchanged
        {
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
        let animated_from = self
            .window_animations
            .get(&window_id)
            .map(WaylandWindowAnimation::displayed_frame);
        let (from, animation_duration) = match mode {
            WindowMoveMode::Snap => {
                self.snap_window_to(window_id, &element, target, target_loc, to_border);
                return;
            }
            WindowMoveMode::AnimateFrom { from, duration } => (from, duration),
            WindowMoveMode::Retarget { duration } => {
                let from = animated_from.unwrap_or_else(|| {
                    let loc = actual_loc.unwrap_or(target_loc);
                    Rect {
                        x: loc.x - from_border,
                        y: loc.y - from_border,
                        w: actual_size.w.max(1),
                        h: actual_size.h.max(1),
                    }
                });
                (from, duration)
            }
        };

        let should_snap = !self.animations_enabled() || from == target;

        if should_snap {
            self.snap_window_to(window_id, &element, target, target_loc, to_border);
            return;
        }

        let now = Instant::now();
        let mut animation = WaylandWindowAnimation::new(
            from,
            target,
            actual_size,
            self.last_configured_size.get(&window_id).copied(),
            animation_duration,
            now,
            from_border,
            to_border,
        );
        if let Some(actual_loc) = actual_loc {
            animation.anchors =
                animation
                    .anchors
                    .continuous_with(from, actual_size, from_border, actual_loc);
        }
        let output_rects: Vec<_> = self
            .space
            .outputs()
            .filter_map(|output| self.space.output_geometry(output))
            .map(|geometry| {
                Rect::new(
                    geometry.loc.x,
                    geometry.loc.y,
                    geometry.size.w,
                    geometry.size.h,
                )
            })
            .collect();
        if let Some(phase) = offscreen_shrink_configure_phase(
            from,
            target,
            actual_size,
            from_border,
            to_border,
            animation.anchors,
            &output_rects,
        ) {
            animation.resize_timing = ResizeTiming::OffscreenShrink;
            animation.resize_configure_phase = phase;
        } else if resize_growth_is_offscreen_at_start(
            from,
            target,
            actual_size,
            from_border,
            animation.anchors,
            &output_rects,
        ) {
            animation.resize_timing = ResizeTiming::OffscreenGrowth;
            animation.resize_configure_phase = OFFSCREEN_GROWTH_CONFIGURE_PHASE;
        }
        let start_loc =
            anchored_surface_location(from, actual_size, from_border, animation.anchors);
        if actual_loc != Some(start_loc) {
            self.remap_element_preserving_z_order(&element, start_loc, false);
        }
        self.placed_border.insert(window_id, from_border);

        self.insert_or_replace_window_animation(window_id, animation);
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
        _now: Instant,
    ) -> Option<Rect> {
        let animation = self.window_animations.remove(&win)?;
        Some(animation.displayed_frame())
    }

    /// Reposition a completed far-anchored transition after a client buffer
    /// commit. Its anchor remains presentation state without keeping the frame
    /// timer active, so a late resize cannot move the preserved edge.
    pub(crate) fn reconcile_completed_window_animation(
        &mut self,
        win: WindowId,
        committed_size: Size<i32, Logical>,
    ) {
        let Some(animation) = self
            .window_animations
            .get(&win)
            .filter(|animation| animation.waiting_for_resize)
        else {
            return;
        };
        let target = animation.target();
        let border_width = animation.target_border_width();
        let anchors = animation.anchors;
        let landed = !animation.needs_landing(committed_size);
        let loc = anchored_surface_location(target, committed_size, border_width, anchors);

        if let Some(element) = self.find_window(win).cloned()
            && self.space.element_location(&element).is_some()
        {
            self.request_visible_window_render(&element);
            if self.space.element_location(&element) != Some(loc) {
                self.remap_element_preserving_z_order(&element, loc, false);
            }
            self.placed_border.insert(win, border_width);
            self.request_visible_window_render(&element);
            self.request_visual_rect_render(target.with_borders(border_width));
        }

        if landed {
            self.last_configured_size
                .insert(win, (target.w.max(1), target.h.max(1)));
            self.window_animations.remove(&win);
        }
    }

    /// Tick all active window animations.
    pub fn tick_animations(&mut self) {
        let preview_active = self.layout_preview_animation.is_active();
        if !self.has_active_window_animations() && !preview_active {
            return;
        }
        let now = Instant::now();
        if preview_active {
            self.layout_preview_animation.tick(now);
            self.request_render();
        }
        let animation_inputs: Vec<_> = self
            .window_animations
            .iter()
            .filter(|(_, animation)| !animation.waiting_for_resize)
            .map(|(&win, _)| {
                (
                    win,
                    self.find_window(win).map(|element| element.geometry().size),
                )
            })
            .collect();
        let output_rects: Vec<_> = if self
            .window_animations
            .values()
            .any(|animation| animation.resize_timing != ResizeTiming::Normal)
        {
            self.space
                .outputs()
                .filter_map(|output| self.space.output_geometry(output))
                .map(|geometry| {
                    Rect::new(
                        geometry.loc.x,
                        geometry.loc.y,
                        geometry.size.w,
                        geometry.size.h,
                    )
                })
                .collect()
        } else {
            Default::default()
        };
        let mut updates = Vec::with_capacity(animation_inputs.len());
        let mut finished: Vec<WindowId> = Vec::new();
        for (win, committed_size) in animation_inputs {
            let Some(committed_size) = committed_size else {
                finished.push(win);
                continue;
            };
            if let Some(animation) = self.window_animations.get_mut(&win) {
                animation.revalidate_offscreen_resize_phase(committed_size, &output_rects);
                let tick = animation.tick(now, committed_size);
                let waiting_for_resize = tick.done && animation.needs_landing(committed_size);
                if tick.done {
                    animation.waiting_for_resize = waiting_for_resize;
                }
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
                    waiting_for_resize,
                ));
            }
        }

        for (
            win,
            loc,
            configure_target,
            previous_frame,
            frame,
            border_width,
            done,
            waiting_for_resize,
        ) in updates
        {
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
                // redraw animated borders even when an anchored edge keeps
                // the surface location stationary.
                self.request_visible_window_render(&element);
                self.request_visual_rect_render(frame.with_borders(border_width));
            } else {
                finished.push(win);
                continue;
            }
            if done && !waiting_for_resize {
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
        self.window_animations
            .values()
            .any(WaylandWindowAnimation::is_active)
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
            None,
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
    fn offscreen_shrink_configures_before_movement_is_complete() {
        let start = Instant::now();
        let from = Rect::new(0, 0, 1000, 1000);
        let target = Rect::new(500, 0, 500, 1000);
        let committed = Size::from((1000, 1000));
        let output = Rect::new(0, 0, 1000, 1000);
        let mut animation = WaylandWindowAnimation::new(
            from,
            target,
            committed,
            None,
            Duration::from_millis(100),
            start,
            0,
            0,
        );

        let phase = offscreen_shrink_configure_phase(
            from,
            target,
            committed,
            0,
            0,
            animation.anchors,
            &[output],
        )
        .expect("the removed half ends outside the output");
        assert!(
            phase > RESIZE_CONFIGURE_PHASE && phase < 1.0,
            "unexpected shrink configure phase: {phase}"
        );
        animation.resize_timing = ResizeTiming::OffscreenShrink;
        animation.resize_configure_phase = phase;
        assert_eq!(
            animation
                .tick(
                    start + Duration::from_secs_f64(0.1 * (phase - 0.01)),
                    committed,
                )
                .configure_size,
            None
        );
        assert_eq!(
            animation
                .tick(start + Duration::from_secs_f64(0.1 * phase), committed)
                .configure_size,
            Some(Size::from((500, 1000)))
        );
    }

    #[test]
    fn overdue_offscreen_shrink_stages_before_landing() {
        let start = Instant::now();
        let from = Rect::new(0, 0, 1000, 1000);
        let target = Rect::new(500, 0, 500, 1000);
        let committed = Size::from((1000, 1000));
        let output = Rect::new(0, 0, 1000, 1000);
        let mut animation = WaylandWindowAnimation::new(
            from,
            target,
            committed,
            None,
            Duration::from_millis(100),
            start,
            0,
            0,
        );
        animation.resize_timing = ResizeTiming::OffscreenShrink;
        animation.resize_configure_phase = offscreen_shrink_configure_phase(
            from,
            target,
            committed,
            0,
            0,
            animation.anchors,
            &[output],
        )
        .unwrap();

        let staged = animation.tick(start + Duration::from_millis(200), committed);
        assert_eq!(staged.configure_size, Some(Size::from((500, 1000))));
        assert!(!staged.done);
        assert_ne!(staged.frame, target);

        let landed = animation.tick(start + Duration::from_millis(200), committed);
        assert_eq!(landed.configure_size, None);
        assert!(landed.done);
        assert_eq!(landed.frame, target);
    }

    #[test]
    fn offscreen_shrink_falls_back_when_integer_rounding_leaves_no_motion() {
        let from = Rect::new(0, 0, 2, 2);
        let target = Rect::new(1, 0, 1, 2);

        assert_eq!(
            offscreen_shrink_configure_phase(
                from,
                target,
                Size::from((2, 2)),
                0,
                0,
                SurfaceAnchors::between(from, target, 0, 0),
                &[Rect::new(0, 0, 2, 2)],
            ),
            None
        );
    }

    #[test]
    fn shrink_stays_mid_animation_when_removed_pixels_touch_an_output() {
        let target = Rect::new(500, 500, 500, 500);
        let committed = Size::from((500, 1000));
        let anchors = SurfaceAnchors::between(Rect::new(500, 0, 500, 1000), target, 0, 0);

        assert!(!resize_removal_is_offscreen_at_target(
            target,
            committed,
            0,
            anchors,
            &[Rect::new(0, 0, 1000, 1000), Rect::new(0, 1000, 1000, 1000)],
        ));
        assert!(!resize_removal_is_offscreen_at_target(
            Rect::new(500, 400, 500, 500),
            committed,
            0,
            anchors,
            &[Rect::new(0, 0, 1000, 1000)],
        ));
    }

    #[test]
    fn offscreen_growth_is_requested_before_movement() {
        let start = Instant::now();
        let from = Rect::new(0, 500, 500, 500);
        let target = Rect::new(0, 0, 500, 1000);
        let committed = Size::from((500, 500));
        let output = Rect::new(0, 0, 1000, 1000);
        let mut animation = WaylandWindowAnimation::new(
            from,
            target,
            committed,
            None,
            Duration::from_millis(100),
            start,
            0,
            0,
        );

        assert!(resize_growth_is_offscreen_at_start(
            from,
            target,
            committed,
            0,
            animation.anchors,
            &[output],
        ));
        animation.resize_configure_phase = OFFSCREEN_GROWTH_CONFIGURE_PHASE;
        assert_eq!(
            animation.tick(start, committed).configure_size,
            Some(Size::from((500, 1000)))
        );
    }

    #[test]
    fn growth_stays_mid_animation_when_new_pixels_touch_an_output() {
        let from = Rect::new(0, 500, 500, 500);
        let target = Rect::new(0, 0, 500, 1000);
        let committed = Size::from((500, 500));
        let anchors = SurfaceAnchors::between(from, target, 0, 0);

        assert!(!resize_growth_is_offscreen_at_start(
            from,
            target,
            committed,
            0,
            anchors,
            &[Rect::new(0, 0, 1000, 1000), Rect::new(0, 1000, 1000, 1000)],
        ));
    }

    #[test]
    fn offscreen_resize_allows_five_percent_visible_leeway() {
        let target = Rect::new(0, 0, 500, 1000);
        let committed = Size::from((500, 500));
        let output = Rect::new(0, 0, 1000, 1000);
        let five_percent_visible = Rect::new(0, 475, 500, 500);
        let more_than_five_percent_visible = Rect::new(0, 474, 500, 500);

        assert!(resize_growth_is_offscreen_at_start(
            five_percent_visible,
            target,
            committed,
            0,
            SurfaceAnchors::between(five_percent_visible, target, 0, 0),
            // Mirrored outputs must not count the visible area twice.
            &[output, output],
        ));
        assert!(!resize_growth_is_offscreen_at_start(
            more_than_five_percent_visible,
            target,
            committed,
            0,
            SurfaceAnchors::between(more_than_five_percent_visible, target, 0, 0),
            &[output],
        ));
    }

    #[test]
    fn offscreen_percentage_handles_maximum_surface_dimensions() {
        assert!(extra_surface_pixels_are_offscreen(
            Rect::new(0, 0, 1, 1),
            Size::from((1, 1)),
            Size::from((i32::MAX, i32::MAX)),
            0,
            SurfaceAnchors {
                x: SurfaceAnchor::Near,
                y: SurfaceAnchor::Near,
            },
            &[Rect::new(0, 0, 2, 1)],
        ));
    }

    #[test]
    fn immediate_growth_falls_back_if_it_becomes_visible_before_the_first_tick() {
        let start = Instant::now();
        let from = Rect::new(0, 500, 500, 500);
        let target = Rect::new(0, 0, 500, 1000);
        let committed = Size::from((500, 500));
        let mut animation = WaylandWindowAnimation::new(
            from,
            target,
            committed,
            None,
            Duration::from_millis(100),
            start,
            0,
            0,
        );
        animation.resize_timing = ResizeTiming::OffscreenGrowth;
        animation.resize_configure_phase = OFFSCREEN_GROWTH_CONFIGURE_PHASE;

        animation.revalidate_offscreen_resize_phase(
            committed,
            &[Rect::new(0, 0, 1000, 1000), Rect::new(0, 1000, 1000, 1000)],
        );

        assert_eq!(animation.resize_configure_phase, RESIZE_CONFIGURE_PHASE);
        assert_eq!(animation.tick(start, committed).configure_size, None);
    }

    #[test]
    fn delayed_shrink_falls_back_after_an_unsafe_late_commit() {
        let start = Instant::now();
        let target = Rect::new(500, 500, 500, 500);
        let initial_committed = Size::from((500, 1000));
        let late_committed = Size::from((500, 400));
        let output = Rect::new(0, 0, 1000, 1000);
        let mut animation = WaylandWindowAnimation::new(
            Rect::new(500, 0, 500, 1000),
            target,
            initial_committed,
            None,
            Duration::from_millis(100),
            start,
            0,
            0,
        );
        animation.resize_timing = ResizeTiming::OffscreenShrink;
        animation.resize_configure_phase = 1.0;

        animation.revalidate_offscreen_resize_phase(late_committed, &[output]);

        assert_eq!(animation.resize_configure_phase, RESIZE_CONFIGURE_PHASE);
        assert_eq!(
            animation
                .tick(start + Duration::from_millis(50), late_committed)
                .configure_size,
            Some(Size::from((500, 500)))
        );
    }

    #[test]
    fn movement_only_transition_never_schedules_a_resize() {
        let start = Instant::now();
        let mut animation = WaylandWindowAnimation::new(
            Rect::new(0, 0, 100, 80),
            Rect::new(50, 20, 100, 80),
            Size::from((100, 80)),
            None,
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
            None,
            Duration::from_millis(100),
            start,
            0,
            0,
        );
        let visual_size_differs_but_client_is_ready = WaylandWindowAnimation::new(
            Rect::new(0, 0, 120, 80),
            Rect::new(10, 0, 140, 80),
            Size::from((140, 80)),
            None,
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
    fn resize_schedule_supersedes_a_stale_outstanding_configure() {
        let animation = WaylandWindowAnimation::new(
            Rect::new(0, 0, 60, 80),
            Rect::new(0, 0, 100, 80),
            Size::from((100, 80)),
            Some((60, 80)),
            Duration::from_millis(100),
            Instant::now(),
            0,
            0,
        );

        assert_eq!(
            animation.resize,
            ResizeConfigure::Pending(Size::from((100, 80)))
        );
    }

    #[test]
    fn borders_interpolate_from_the_displayed_to_the_target_width() {
        let start = Instant::now();
        let mut animation = WaylandWindowAnimation::new(
            Rect::new(0, 30, 1200, 740),
            Rect::new(150, 126, 896, 573),
            Size::from((1200, 740)),
            None,
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

        // Border and buffer-size presentation are independent. The horizontal
        // Horizontal edges move equally, so that axis follows the near edge.
        // Vertically, the farther-moving near edge carries the surface while
        // the border follows the eased frame progress.
        let midpoint = animation.tick(start + Duration::from_millis(50), Size::from((1200, 740)));
        assert_eq!(midpoint.surface_location, Point::from((133, 116)));
        assert_eq!(animation.displayed_border_width(), 2);

        // The transition ends fully bordered, landed exactly on the target
        // inner rectangle: content origin offset by the final border width.
        let tick = animation.tick(start + Duration::from_millis(100), Size::from((896, 573)));
        assert_eq!(tick.surface_location, Point::from((152, 128)));
        assert_eq!(animation.displayed_border_width(), 2);
        assert_eq!(animation.target_border_width(), 2);
    }

    #[test]
    fn centered_float_to_single_tile_anchors_near_edges_so_the_surface_moves() {
        let start = Instant::now();
        let from = Rect::new(198, 313, 600, 400);
        let to = Rect::new(0, 30, 1000, 970);
        let committed = Size::from((600, 400));
        let mut animation = WaylandWindowAnimation::new(
            from,
            to,
            committed,
            None,
            Duration::from_millis(100),
            start,
            2,
            0,
        );

        assert_eq!(animation.anchors.x, SurfaceAnchor::Near);
        assert_eq!(animation.anchors.y, SurfaceAnchor::Near);

        let midpoint = animation.tick(start + Duration::from_millis(50), committed);
        assert!(midpoint.surface_location.x < from.x + 2);
        assert!(midpoint.surface_location.y < from.y + 2);

        let end = animation.tick(start + Duration::from_millis(100), Size::from((1000, 970)));
        assert_eq!(end.surface_location, Point::from((to.x, to.y)));
    }

    #[test]
    fn right_half_to_bottom_right_quarter_follows_the_moving_top_edge() {
        let animation = WaylandWindowAnimation::new(
            Rect::new(500, 0, 500, 1000),
            Rect::new(500, 500, 500, 500),
            Size::from((500, 1000)),
            None,
            Duration::from_millis(100),
            Instant::now(),
            0,
            0,
        );

        assert_eq!(animation.anchors.x, SurfaceAnchor::Near);
        assert_eq!(animation.anchors.y, SurfaceAnchor::Near);
    }

    #[test]
    fn bottom_left_quarter_to_left_half_follows_the_moving_top_edge() {
        let start = Instant::now();
        let committed = Size::from((500, 500));
        let mut animation = WaylandWindowAnimation::new(
            Rect::new(0, 500, 500, 500),
            Rect::new(0, 0, 500, 1000),
            committed,
            None,
            Duration::from_millis(100),
            start,
            0,
            0,
        );

        assert_eq!(animation.anchors.y, SurfaceAnchor::Near);
        let midpoint = animation.tick(start + Duration::from_millis(50), committed);
        assert!(midpoint.surface_location.y < 500);
    }

    #[test]
    fn retarget_keeps_the_anchor_matching_the_current_surface_location() {
        let frame = Rect::new(0, 0, 120, 80);
        let committed = Size::from((100, 80));
        let current_location = Point::from((20, 0));
        let preferred = SurfaceAnchors {
            x: SurfaceAnchor::Near,
            y: SurfaceAnchor::Near,
        };

        assert_eq!(
            preferred.continuous_with(frame, committed, 0, current_location),
            SurfaceAnchors {
                x: SurfaceAnchor::Far,
                y: SurfaceAnchor::Near,
            }
        );
    }

    #[test]
    fn one_sided_growth_follows_the_moving_far_edge() {
        let anchors =
            SurfaceAnchors::between(Rect::new(0, 0, 100, 80), Rect::new(0, 0, 140, 80), 0, 0);
        let halfway_frame = Rect::new(0, 0, 120, 80);

        let before_resize =
            anchored_surface_location(halfway_frame, Size::from((100, 80)), 0, anchors);
        let after_resize =
            anchored_surface_location(halfway_frame, Size::from((140, 80)), 0, anchors);

        assert_eq!(anchors.x, SurfaceAnchor::Far);
        assert_eq!(before_resize, Point::from((20, 0)));
        assert_eq!(after_resize, Point::from((-20, 0)));
        assert_eq!(
            anchored_surface_location(Rect::new(0, 0, 140, 80), Size::from((140, 80)), 0, anchors,),
            Point::from((0, 0))
        );
    }

    #[test]
    fn one_sided_shrink_follows_the_moving_near_edge() {
        let anchors =
            SurfaceAnchors::between(Rect::new(0, 0, 100, 80), Rect::new(40, 0, 60, 80), 0, 0);
        let halfway_frame = Rect::new(20, 0, 80, 80);

        let before_resize =
            anchored_surface_location(halfway_frame, Size::from((100, 80)), 0, anchors);
        let after_resize =
            anchored_surface_location(halfway_frame, Size::from((60, 80)), 0, anchors);

        assert_eq!(anchors.x, SurfaceAnchor::Near);
        assert_eq!(before_resize, Point::from((20, 0)));
        assert_eq!(after_resize, Point::from((20, 0)));
        assert_eq!(
            anchored_surface_location(Rect::new(40, 0, 60, 80), Size::from((60, 80)), 0, anchors,),
            Point::from((40, 0))
        );
    }

    #[test]
    fn far_anchored_completion_waits_for_the_target_committed_size() {
        let animation = WaylandWindowAnimation::new(
            Rect::new(0, 0, 100, 80),
            Rect::new(0, 0, 140, 80),
            Size::from((100, 80)),
            None,
            Duration::from_millis(100),
            Instant::now(),
            0,
            0,
        );

        assert_eq!(animation.anchors.x, SurfaceAnchor::Far);
        assert!(animation.needs_landing(Size::from((100, 80))));
        assert!(!animation.needs_landing(Size::from((140, 80))));
    }

    #[test]
    fn every_edge_combination_anchors_the_farther_traveled_edge() {
        let border = 4;
        let from = Rect::new(0, 0, 100, 80);

        for left_delta in [-20, 0, 20] {
            for right_delta in [-20, 0, 20] {
                for top_delta in [-20, 0, 20] {
                    for bottom_delta in [-20, 0, 20] {
                        let left = left_delta;
                        let right = 100 + right_delta;
                        let top = top_delta;
                        let bottom = 80 + bottom_delta;
                        let frame = Rect::new(left, top, right - left, bottom - top);
                        assert!(frame.is_valid());
                        let anchors = SurfaceAnchors::between(from, frame, border, border);
                        assert_eq!(
                            anchors.x,
                            if left_delta.abs() >= right_delta.abs() {
                                SurfaceAnchor::Near
                            } else {
                                SurfaceAnchor::Far
                            }
                        );
                        assert_eq!(
                            anchors.y,
                            if top_delta.abs() >= bottom_delta.abs() {
                                SurfaceAnchor::Near
                            } else {
                                SurfaceAnchor::Far
                            }
                        );

                        let committed_sizes = [
                            Size::from((100, 80)),
                            Size::from((frame.w, frame.h)),
                            Size::from((120, 60)),
                        ];

                        for committed in committed_sizes {
                            let location =
                                anchored_surface_location(frame, committed, border, anchors);
                            match anchors.x {
                                SurfaceAnchor::Near => assert_eq!(location.x, frame.x + border),
                                SurfaceAnchor::Far => {
                                    assert_eq!(location.x + committed.w, frame.x + border + frame.w)
                                }
                            }
                            match anchors.y {
                                SurfaceAnchor::Near => assert_eq!(location.y, frame.y + border),
                                SurfaceAnchor::Far => {
                                    assert_eq!(location.y + committed.h, frame.y + border + frame.h)
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
