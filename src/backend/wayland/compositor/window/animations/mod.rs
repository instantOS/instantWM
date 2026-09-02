use smithay::utils::{Logical, Point, Size};
use std::time::{Duration, Instant};

use crate::backend::wayland::compositor::WaylandState;
use crate::constants::animation::WAYLAND_DEFAULT_ANIMATION_MILLIS;
use crate::types::{Rect, WindowId};

mod transition;

pub(crate) use transition::WaylandWindowAnimation;

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

impl WaylandState {
    pub(crate) fn set_layout_preview_target(
        &mut self,
        target: Option<Rect>,
        style: crate::types::InteractionOutlineStyle,
        target_window: Option<WindowId>,
        animate: bool,
        duration: Duration,
    ) {
        self.layout_preview_style = style;
        self.layout_preview_target = target_window;
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

    pub(crate) fn layout_preview_target(&self) -> Option<WindowId> {
        self.layout_preview_target
    }

    pub(crate) fn has_active_layout_preview_animation(&self) -> bool {
        self.layout_preview_animation.is_active()
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
            .is_some_and(|state| state.interaction.drag.active_interaction().is_some())
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

    fn output_rects(&self) -> Vec<Rect> {
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
            animation.preserve_surface_continuity(actual_size, actual_loc);
        }
        animation.prepare_resize_timing(&self.output_rects());
        let start_loc = animation.displayed_surface_location(actual_size);
        if actual_loc != Some(start_loc) {
            self.remap_element_preserving_z_order(&element, start_loc, false);
        }
        self.placed_border.insert(window_id, from_border);

        self.window_animations.insert(window_id, animation);
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
            if anim.requires_resize() {
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
            .filter(|animation| animation.is_waiting_for_resize())
        else {
            return;
        };
        let target = animation.target();
        let border_width = animation.target_border_width();
        let landed = animation.has_landed(committed_size);
        let loc = animation.target_surface_location(committed_size);

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
            .filter(|(_, animation)| !animation.is_waiting_for_resize())
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
            .any(WaylandWindowAnimation::requires_output_revalidation)
        {
            self.output_rects()
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
                    tick.waiting_for_resize,
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

    /// Check whether one window currently has an active geometry animation.
    pub fn window_has_active_animation(&self, win: WindowId) -> bool {
        self.window_animations
            .get(&win)
            .is_some_and(WaylandWindowAnimation::is_active)
    }

    /// Check if any compositor visual transition needs another frame.
    pub fn has_active_animations(&self) -> bool {
        self.has_active_window_animations()
            || self.layout_preview_animation.is_active()
            || self.shortcut_recovery_needs_tick()
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
