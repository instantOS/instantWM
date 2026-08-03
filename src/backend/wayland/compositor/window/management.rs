use smithay::utils::{Point, Rectangle};

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::compositor::window::animations::WindowMoveMode;
use crate::types::{Rect, WindowId};

impl WaylandState {
    /// Re-map an already-mapped element without changing its relative z-order.
    ///
    /// Smithay's `map_element` updates the location but also raises the element.
    /// Layout code uses remaps for geometry changes, so we use `relocate_element`
    /// (which leaves stacking and active state untouched) for already-mapped
    /// elements and fall back to `map_element` for the first mapping.
    pub(crate) fn remap_element_preserving_z_order(
        &mut self,
        element: &smithay::desktop::Window,
        location: Point<i32, smithay::utils::Logical>,
        activate: bool,
    ) {
        if self.space.element_location(element).is_some() {
            self.space.relocate_element(element, location);
        } else {
            self.space.map_element(element.clone(), location, activate);
        }
    }

    /// Apply authoritative geometry from the WM layer.
    ///
    /// The WM already decided whether a move animates and routes animated
    /// transitions through [`set_window_target_rect`] with an explicit
    /// animation mode. Anything reaching this entry point is geometry the WM
    /// wants applied now, so it always snaps rather than re-deriving an
    /// animation mode from the active-drag heuristic.
    pub fn resize_window(&mut self, window: WindowId, rect: Rect) {
        if let Some(pending) = self.pending_authoritative_sizes.get_mut(&window) {
            *pending = (rect.w.max(1), rect.h.max(1));
        }
        if let Some(element) = self.find_window(window).cloned()
            && let Some(surface) = element.x11_surface()
        {
            let geometry = Rectangle::new(
                (rect.x, rect.y).into(),
                (rect.w.max(1), rect.h.max(1)).into(),
            );
            let _ = surface.configure(Some(geometry));
        }
        let mode = WindowMoveMode::Snap;
        self.set_window_target_rect(window, rect, mode);
    }

    /// Apply an authoritative presentation-transition rectangle.
    ///
    /// Until the client commits this size, buffers from the previous
    /// presentation must not feed back into logical model geometry.
    pub(crate) fn configure_presentation_transition(&mut self, window: WindowId, rect: Rect) {
        self.pending_authoritative_sizes
            .insert(window, (rect.w.max(1), rect.h.max(1)));
        self.resize_window(window, rect);
    }

    /// Raise a window to the top of the stack.
    pub fn raise_window_visual_only(&mut self, window: WindowId) {
        if let Some(element) = self.find_window(window).cloned() {
            // Focus is handled independently by `set_focus`, so we pass `false`
            self.space.raise_element(&element, false);

            // XWayland requires us to explicitly raise the X11 surface so X clients draw correctly.
            if let Some(surface) = element.x11_surface()
                && let Some(xwm) = self.xwm.as_mut()
            {
                let _ = xwm.raise_window(surface);
            }
        }
        self.raise_unmanaged_x11_windows();
    }

    /// Apply a complete z-order (bottom-to-top).
    pub fn apply_z_order(&mut self, windows: &[WindowId]) {
        for window in windows.iter() {
            if let Some(element) = self.find_window(*window).cloned() {
                // Focus / activation is managed by `set_focus`, so we pass `false`
                // here to avoid overriding the focus state visually.
                self.space.raise_element(&element, false);
            }
        }
        self.raise_unmanaged_x11_windows();
    }
}
