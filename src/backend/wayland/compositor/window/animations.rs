use smithay::desktop::Window;
use smithay::utils::{Logical, Point};
use std::time::{Duration, Instant};

use crate::backend::wayland::compositor::WaylandState;
use crate::types::WindowId;

/// Window animation state.
#[derive(Debug, Clone, Copy)]
pub struct WaylandWindowAnimation {
    pub(crate) from: Point<i32, Logical>,
    pub(crate) to: Point<i32, Logical>,
    pub(crate) started_at: Instant,
    pub(crate) duration: Duration,
}

impl WaylandState {
    pub(crate) fn animations_enabled(&self) -> bool {
        self.globals().map(|g| g.behavior.animated).unwrap_or(false)
    }

    pub(crate) fn interactive_motion_active(&self) -> bool {
        self.globals()
            .map(|g| g.drag.interactive.active && g.drag.interactive.dragging)
            .unwrap_or(false)
    }

    pub(crate) fn set_window_target_location(
        &mut self,
        window_id: WindowId,
        element: Window,
        target: Point<i32, Logical>,
        remap: bool,
    ) {
        // Use the client's stored geometry as the authoritative current position
        // to avoid animating from stale locations after map/unmap cycles.
        let actual_loc = self.space.element_location(&element);

        // Do not update the location if it is visually already at the target
        // and we don't forcefully want to remap, to prevent unnecessary Z-order pops.
        if actual_loc == Some(target) && !remap {
            self.window_animations.remove(&window_id);
            return;
        }

        // Use the client's stored geometry as the authoritative current position
        // to avoid animating from stale locations after map/unmap cycles.
        let current = actual_loc
            .or_else(|| {
                self.globals().and_then(|g| {
                    g.clients
                        .get(&window_id)
                        .map(|c| Point::from((c.geo.x + c.border_width, c.geo.y + c.border_width)))
                })
            })
            .unwrap_or(target);

        if !self.animations_enabled() || remap || current == target {
            self.window_animations.remove(&window_id);
            // In Smithay, activate=true steals visual focus. instantWM manages focus via `set_focus()`.
            self.space.map_element(element, target, false);
            return;
        }

        if self
            .window_animations
            .get(&window_id)
            .is_some_and(|anim| anim.to == target)
        {
            return;
        }

        self.window_animations.insert(
            window_id,
            WaylandWindowAnimation {
                from: current,
                to: target,
                started_at: Instant::now(),
                duration: Duration::from_millis(90),
            },
        );
    }

    /// Tick all active window animations.
    pub fn tick_window_animations(&mut self) {
        if self.window_animations.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut updates: Vec<(WindowId, Point<i32, Logical>, bool)> = Vec::new();
        for (win, anim) in &self.window_animations {
            let elapsed = now.saturating_duration_since(anim.started_at);
            let raw_t = (elapsed.as_secs_f64() / anim.duration.as_secs_f64()).clamp(0.0, 1.0);
            let t = crate::animation::ease_out_cubic(raw_t);
            let x = anim.from.x + ((anim.to.x - anim.from.x) as f64 * t).round() as i32;
            let y = anim.from.y + ((anim.to.y - anim.from.y) as f64 * t).round() as i32;
            updates.push((*win, Point::from((x, y)), raw_t >= 1.0));
        }

        let mut finished: Vec<WindowId> = Vec::new();
        for (win, loc, done) in updates {
            if let Some(element) = self.find_window(win).cloned() {
                self.space.map_element(element, loc, false);
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

    /// Check if there are active window animations.
    pub fn has_active_window_animations(&self) -> bool {
        !self.window_animations.is_empty()
    }
}
