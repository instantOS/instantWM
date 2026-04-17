use smithay::utils::{Logical, Point};
use std::time::{Duration, Instant};

use crate::backend::wayland::compositor::WaylandState;
use crate::constants::animation::WAYLAND_DEFAULT_ANIMATION_MILLIS;
use crate::types::{Rect, WindowId};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum WindowMoveMode {
    AnimateTo,
    Immediate,
    AnimateFrom { from: Rect, duration: Duration },
}

impl WaylandState {
    fn insert_or_replace_window_animation(
        &mut self,
        window_id: WindowId,
        from: Rect,
        to: Rect,
        duration: Duration,
    ) {
        self.window_animations.insert(
            window_id,
            crate::animation::WindowAnimation {
                from,
                to,
                started_at: Instant::now(),
                duration,
            },
        );
    }

    pub(crate) fn drop_window_animation(&mut self, win: WindowId) {
        self.window_animations.remove(&win);
    }

    pub(crate) fn animations_enabled(&self) -> bool {
        self.globals().map(|g| g.behavior.animated).unwrap_or(false)
    }

    pub(crate) fn interactive_motion_active(&self) -> bool {
        self.globals()
            .map(|g| g.drag.interactive.active && g.drag.interactive.dragging)
            .unwrap_or(false)
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

    /// Place a window at `target` (in outer/WM coordinates) using the given
    /// movement mode.
    ///
    /// This is a **visual placement** function.  It converts the outer WM
    /// rect to inner/surface coordinates (adding `border_width`), sends a
    /// configure if the size changed, and either snaps or animates the
    /// element to the target location.
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
        let Some(border_width) = self
            .globals()
            .and_then(|g| g.clients.get(&window_id).map(|c| c.border_width))
        else {
            return;
        };

        // Convert outer WM rect → inner surface rect.
        let target_loc: Point<i32, Logical> =
            Point::from((target.x + border_width, target.y + border_width));
        let target_inner = Rect {
            x: target_loc.x,
            y: target_loc.y,
            w: target.w,
            h: target.h,
        };

        let actual_loc = self.space.element_location(&element);

        // Keep an in-flight animation when callers repeatedly request the
        // same target (e.g. sync_space_from_globals during a decorative
        // AnimateFrom slide-in).
        if mode == WindowMoveMode::AnimateTo
            && self
                .window_animations
                .get(&window_id)
                .is_some_and(|anim| anim.to == target_inner)
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

        // Send a configure if the size changed.
        if element.toplevel().is_some() {
            let configured = (target.w.max(1), target.h.max(1));
            let unchanged = self
                .last_configured_size
                .get(&window_id)
                .is_some_and(|&size| size == configured);
            if !unchanged {
                let size = smithay::utils::Size::<i32, smithay::utils::Logical>::new(
                    configured.0,
                    configured.1,
                );
                self.send_toplevel_configure(&element, Some(size));
                self.last_configured_size.insert(window_id, configured);
            }
        }

        // Resolve the animation start position.
        let (from_inner, animation_duration) = match mode {
            WindowMoveMode::AnimateFrom { from, duration } => (
                Rect {
                    x: from.x + border_width,
                    y: from.y + border_width,
                    w: from.w,
                    h: from.h,
                },
                duration,
            ),
            WindowMoveMode::AnimateTo | WindowMoveMode::Immediate => {
                let loc = actual_loc.unwrap_or(target_loc);
                (
                    Rect {
                        x: loc.x,
                        y: loc.y,
                        w: target.w,
                        h: target.h,
                    },
                    Duration::from_millis(WAYLAND_DEFAULT_ANIMATION_MILLIS),
                )
            }
        };

        let should_snap = !self.animations_enabled()
            || mode == WindowMoveMode::Immediate
            || (from_inner.x == target_loc.x && from_inner.y == target_loc.y);

        if should_snap {
            self.remap_window_immediately(window_id, &element, target_loc);
            return;
        }

        // For decorative slide-ins, place element at the start position so
        // the first rendered frame is correct.
        if matches!(mode, WindowMoveMode::AnimateFrom { .. }) {
            self.remap_element_preserving_z_order(
                &element,
                Point::from((from_inner.x, from_inner.y)),
                false,
            );
        }

        self.insert_or_replace_window_animation(
            window_id,
            from_inner,
            target_inner,
            animation_duration,
        );
    }

    pub fn active_window_animation_count(&self) -> usize {
        self.window_animations.len()
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
            let loc = Point::from((anim.to.x, anim.to.y));
            self.remap_element_preserving_z_order(&element, loc, false);
        }
    }

    /// Tick all active window animations.
    pub fn tick_window_animations(&mut self) {
        if self.window_animations.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut updates: Vec<(WindowId, Point<i32, Logical>, bool)> = Vec::new();
        for (win, anim) in &self.window_animations {
            let tick = anim.tick(now);
            updates.push((*win, Point::from((tick.rect.x, tick.rect.y)), tick.done));
        }

        let mut finished: Vec<WindowId> = Vec::new();
        for (win, loc, done) in updates {
            if let Some(element) = self.find_window(win).cloned() {
                self.remap_element_preserving_z_order(&element, loc, false);
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

    /// Check if the window has an in-flight animation heading toward `outer_target`
    /// (in WM/outer coordinates).  The border-width conversion is handled
    /// internally so callers don't need to know about the inner coordinate space.
    pub(crate) fn animation_targets_outer_rect(&self, win: WindowId, outer_target: Rect) -> bool {
        let Some(anim) = self.window_animations.get(&win) else {
            return false;
        };
        let border_width = self
            .globals()
            .and_then(|g| g.clients.get(&win).map(|c| c.border_width))
            .unwrap_or(0);
        let inner_target = Rect {
            x: outer_target.x + border_width,
            y: outer_target.y + border_width,
            w: outer_target.w,
            h: outer_target.h,
        };
        anim.to == inner_target
    }
}
