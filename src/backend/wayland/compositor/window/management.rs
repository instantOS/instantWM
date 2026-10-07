use crate::backend::wayland::compositor::WaylandNativeState;
use smithay::utils::Point;

use crate::backend::wayland::compositor::window::animations::WindowMoveMode;
use crate::types::{Rect, WindowId};

impl WaylandNativeState {
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
        // Compositor-owned moves need damage even when the client never
        // commits another buffer. Include both the vacated and new outputs.
        self.request_visible_window_render(element);
        if self.space.element_location(element).is_some() {
            self.space.relocate_element(element, location);
        } else {
            self.space.map_element(element.clone(), location, activate);
        }
        self.request_visible_window_render(element);
    }

    /// Apply authoritative geometry from the WM layer.
    ///
    /// The WM already decided whether a move animates and routes animated
    /// transitions through [`set_window_target_rect`] with an explicit
    /// animation mode. Anything reaching this entry point is geometry the WM
    /// wants applied now, so it always snaps rather than re-deriving an
    /// animation mode from the active-drag heuristic.
    pub fn resize_window(
        &mut self,
        core_view: &crate::core_state::CoreState,
        window: WindowId,
        rect: Rect,
    ) {
        let mode = WindowMoveMode::Snap;
        // The entry point resolves the managed client once: unmanaged windows
        // must not be visually placed, but still receive their protocol
        // resize below.
        if let Some(border_width) = core_view.model.client(window).map(|c| c.border_width) {
            self.set_window_target_rect(core_view, window, rect, border_width, mode);
        }
        // An immediate request may preserve an existing spatial animation to
        // this target, but its protocol resize must still be sent now.
        if let Some(element) = self.find_window(window).cloned() {
            self.dispatch_window_resize(core_view, window, &element, rect);
        }
    }

    /// Apply a complete z-order (bottom-to-top).
    pub fn apply_z_order(&mut self, windows: &[WindowId]) {
        for window in windows.iter() {
            if let Some(element) = self.find_window(*window).cloned() {
                // Focus / activation is managed by `set_focus`, so we pass `false`
                // here to avoid overriding the focus state visually.
                self.request_visible_window_render(&element);
                self.space.raise_element(&element, false);
            }
        }
        let x11_order: Vec<_> = windows
            .iter()
            .filter_map(|win| {
                self.find_window(*win)
                    .and_then(|window| window.x11_surface())
                    .cloned()
            })
            .collect();
        if let Some(xwm) = self.xwm.as_mut()
            && let Err(error) = xwm.update_stacking_order_downwards(x11_order.iter())
        {
            log::warn!("failed to synchronize Xwayland stacking: {error}");
        }
        self.raise_unmanaged_x11_windows();
    }
}
