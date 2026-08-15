use smithay::utils::{Logical, Point, Size};
use std::time::{Duration, Instant};

use crate::animation::{WindowAnimation, ease_out_cubic};
use crate::types::Rect;

mod offscreen;

use offscreen::{offscreen_shrink_configure_phase, resize_growth_is_offscreen_at_start};

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
    frame: WindowAnimation,
    displayed_frame: Rect,
    displayed_border: i32,
    from_border: i32,
    to_border: i32,
    committed_size_at_start: Size<i32, Logical>,
    anchors: SurfaceAnchors,
    resize: ResizeConfigure,
    resize_timing: ResizeTiming,
    resize_configure_phase: f64,
    shrink_stage_presented: bool,
    waiting_for_resize: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct WaylandAnimationTick {
    pub(super) previous_frame: Rect,
    pub(super) frame: Rect,
    pub(super) surface_location: Point<i32, Logical>,
    pub(super) configure_size: Option<Size<i32, Logical>>,
    pub(super) done: bool,
    pub(super) waiting_for_resize: bool,
}

/// Blend a border width across eased animation progress so the border keeps
/// pace with the frame motion.
fn interpolate_borders(from: i32, to: i32, eased: f64) -> i32 {
    (from as f64 + (to as f64 - from as f64) * eased).round() as i32
}

impl WaylandWindowAnimation {
    pub(super) fn new(
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
            frame: WindowAnimation {
                from,
                to,
                started_at: now,
                duration,
            },
            displayed_frame: from,
            displayed_border: from_border,
            from_border,
            to_border,
            committed_size_at_start: committed_size,
            anchors,
            resize: ResizeConfigure::toward(committed_size, last_configured_size, to),
            resize_timing: ResizeTiming::Normal,
            resize_configure_phase: RESIZE_CONFIGURE_PHASE,
            shrink_stage_presented: false,
            waiting_for_resize: false,
        }
    }

    pub(super) fn target(&self) -> Rect {
        self.frame.to
    }

    pub(super) fn displayed_frame(&self) -> Rect {
        self.displayed_frame
    }

    /// The border width currently presented for this window: the interpolated
    /// value at the latest frame, `from_border` before the first tick and
    /// `to_border` once the transition is complete.
    pub(super) fn displayed_border_width(&self) -> i32 {
        self.displayed_border
    }

    /// The border width the final placement requests.
    pub(super) fn target_border_width(&self) -> i32 {
        self.to_border
    }

    pub(super) fn requires_resize(&self) -> bool {
        !matches!(self.resize, ResizeConfigure::Unchanged)
    }

    fn needs_landing(&self, committed_size: Size<i32, Logical>) -> bool {
        let target = self.target();
        (self.anchors.x == SurfaceAnchor::Far && committed_size.w != target.w)
            || (self.anchors.y == SurfaceAnchor::Far && committed_size.h != target.h)
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.waiting_for_resize
    }

    pub(super) fn has_landed(&self, committed_size: Size<i32, Logical>) -> bool {
        !self.needs_landing(committed_size)
    }

    pub(super) fn is_waiting_for_resize(&self) -> bool {
        self.waiting_for_resize
    }

    pub(super) fn requires_output_revalidation(&self) -> bool {
        self.resize_timing != ResizeTiming::Normal
    }

    pub(super) fn preserve_surface_continuity(
        &mut self,
        committed_size: Size<i32, Logical>,
        current_location: Point<i32, Logical>,
    ) {
        self.anchors = self.anchors.continuous_with(
            self.displayed_frame,
            committed_size,
            self.displayed_border,
            current_location,
        );
    }

    pub(super) fn displayed_surface_location(
        &self,
        committed_size: Size<i32, Logical>,
    ) -> Point<i32, Logical> {
        anchored_surface_location(
            self.displayed_frame,
            committed_size,
            self.displayed_border,
            self.anchors,
        )
    }

    pub(super) fn target_surface_location(
        &self,
        committed_size: Size<i32, Logical>,
    ) -> Point<i32, Logical> {
        anchored_surface_location(self.target(), committed_size, self.to_border, self.anchors)
    }

    pub(super) fn prepare_resize_timing(&mut self, outputs: &[Rect]) {
        if let Some(phase) = offscreen_shrink_configure_phase(
            self.frame.from,
            self.target(),
            self.committed_size_at_start,
            self.from_border,
            self.to_border,
            self.anchors,
            outputs,
        ) {
            self.resize_timing = ResizeTiming::OffscreenShrink;
            self.resize_configure_phase = phase;
        } else if resize_growth_is_offscreen_at_start(
            self.frame.from,
            self.target(),
            self.committed_size_at_start,
            self.from_border,
            self.anchors,
            outputs,
        ) {
            self.resize_timing = ResizeTiming::OffscreenGrowth;
            self.resize_configure_phase = OFFSCREEN_GROWTH_CONFIGURE_PHASE;
        }
    }

    pub(super) fn revalidate_offscreen_resize_phase(
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

    pub(super) fn tick(
        &mut self,
        now: Instant,
        committed_size: Size<i32, Logical>,
    ) -> WaylandAnimationTick {
        let previous_frame = self.displayed_frame;
        let mut tick = self.frame.tick(now);
        let should_present_shrink_stage = self.resize_timing == ResizeTiming::OffscreenShrink
            && !self.shrink_stage_presented
            && matches!(self.resize, ResizeConfigure::Pending(_))
            && tick.progress >= self.resize_configure_phase;
        if should_present_shrink_stage {
            tick.progress = self.resize_configure_phase;
            tick.rect = self.frame.sample(tick.progress).rect;
            tick.done = false;
            self.shrink_stage_presented = true;
        }
        let eased = ease_out_cubic(tick.progress);
        self.displayed_frame = tick.rect;
        self.displayed_border = interpolate_borders(self.from_border, self.to_border, eased);
        if tick.done {
            self.waiting_for_resize = self.needs_landing(committed_size);
        }
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
            waiting_for_resize: self.waiting_for_resize,
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

#[cfg(test)]
mod tests;
