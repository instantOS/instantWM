use smithay::utils::{Logical, Point};
use std::time::{Duration, Instant};

use crate::backend::wayland::compositor::WaylandState;
use crate::constants::animation::WAYLAND_DEFAULT_ANIMATION_MILLIS;
use crate::types::{Rect, WindowId};

pub type WaylandWindowAnimation = crate::animation::WindowAnimation;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum WindowMoveMode {
    AnimateTo,
    Immediate,
    AnimateFrom { from: Rect, duration: Duration },
}

impl WaylandState {
    fn insert_window_animation(
        &mut self,
        window_id: WindowId,
        from: Rect,
        to: Rect,
        duration: Duration,
    ) {
        self.window_animations.insert(
            window_id,
            WaylandWindowAnimation {
                from,
                to,
                started_at: Instant::now(),
                duration,
            },
        );
    }

    pub(crate) fn animations_enabled(&self) -> bool {
        self.globals().map(|g| g.behavior.animated).unwrap_or(false)
    }

    pub(crate) fn interactive_motion_active(&self) -> bool {
        self.globals()
            .map(|g| g.drag.interactive.active && g.drag.interactive.dragging)
            .unwrap_or(false)
    }

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

        let target_loc: Point<i32, Logical> =
            Point::from((target.x + border_width, target.y + border_width));
        let actual_loc = self.space.element_location(&element);

        // Geometry updates for hidden/unmapped windows must not remap them as a
        // side effect. The behavioral layer owns visibility; the backend should
        // only move windows that are already mapped (or are being interactively
        // remapped on purpose).
        if actual_loc.is_none() && mode != WindowMoveMode::Immediate {
            self.window_animations.remove(&window_id);
            return;
        }

        // Do not update the location if it is visually already at the target
        // and we don't forcefully want to remap, to prevent unnecessary Z-order pops.
        // However, a size-only change (e.g. mfact adjustment) still needs a
        // configure even when x,y are unchanged.
        let configured_size = (target.w.max(1), target.h.max(1));
        let size_unchanged = self
            .last_configured_size
            .get(&window_id)
            .is_some_and(|&size| size == configured_size);
        if actual_loc == Some(target_loc) && mode == WindowMoveMode::AnimateTo && size_unchanged {
            self.window_animations.remove(&window_id);
            return;
        }

        let (from_rect, animation_duration) = match mode {
            WindowMoveMode::AnimateFrom { from, duration } => (
                Some(Rect {
                    x: from.x + border_width,
                    y: from.y + border_width,
                    w: from.w,
                    h: from.h,
                }),
                duration,
            ),
            WindowMoveMode::AnimateTo | WindowMoveMode::Immediate => {
                (None, Duration::from_millis(WAYLAND_DEFAULT_ANIMATION_MILLIS))
            }
        };

        // Use the client's stored geometry as the authoritative current position
        // to avoid animating from stale locations after map/unmap cycles.
        let current = from_rect.unwrap_or_else(|| {
            actual_loc
                .map(|loc| Rect {
                    x: loc.x,
                    y: loc.y,
                    w: target.w,
                    h: target.h,
                })
                .or_else(|| {
                    self.globals().and_then(|g| {
                        g.clients.get(&window_id).map(|c| Rect {
                            x: c.geo.x + c.border_width,
                            y: c.geo.y + c.border_width,
                            w: c.geo.w,
                            h: c.geo.h,
                        })
                    })
                })
                .unwrap_or(Rect {
                    x: target_loc.x,
                    y: target_loc.y,
                    w: target.w,
                    h: target.h,
                })
        });

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

        let should_remap_immediately = !self.animations_enabled()
            || mode == WindowMoveMode::Immediate
            || (current.x == target_loc.x && current.y == target_loc.y);

        if should_remap_immediately {
            self.window_animations.remove(&window_id);
            // In Smithay, activate=true steals visual focus. instantWM manages focus via `set_focus()`.
            self.remap_element_preserving_z_order(&element, target_loc, false);
            return;
        }

        if let Some(from) = from_rect {
            // For decorative slide-ins the WM should already treat the client
            // as living at `target`, but the compositor still needs the mapped
            // element to start from the off-screen location so the first
            // rendered frame is visible and the animation direction is correct.
            self.remap_element_preserving_z_order(&element, Point::from((from.x, from.y)), false);
        }

        self.insert_window_animation(
            window_id,
            current,
            Rect {
                x: target_loc.x,
                y: target_loc.y,
                w: target.w,
                h: target.h,
            },
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
            let tick = crate::animation::interpolate_animation_tick(anim, now);
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
            self.window_animations.remove(&win);
        }
    }

    /// Cancel all in-flight window animations, snapping each mapped window
    /// to its animation target position.
    pub fn cancel_all_window_animations(&mut self) {
        let finished: Vec<(WindowId, Rect)> = self
            .window_animations
            .drain()
            .map(|(win, anim)| (win, anim.to))
            .collect();
        for (win, target) in finished {
            if let Some(element) = self.find_window(win).cloned()
                && self.space.element_location(&element).is_some()
            {
                let loc = Point::from((target.x, target.y));
                self.remap_element_preserving_z_order(&element, loc, false);
            }
        }
    }

    /// Check if there are active window animations.
    pub fn has_active_window_animations(&self) -> bool {
        !self.window_animations.is_empty()
    }
}
