use smithay::utils::Point;

use crate::backend::wayland::compositor::WaylandState;
use crate::types::{Rect, WindowId};

impl WaylandState {
    /// Re-map an already-mapped element without changing its relative z-order.
    ///
    /// Smithay's `map_element` updates the location but also raises the element.
    /// Layout code uses remaps for geometry changes, so preserve the previous
    /// stacking order here to keep overlap semantics controlled by WM restacks.
    pub(crate) fn remap_element_preserving_z_order(
        &mut self,
        element: &smithay::desktop::Window,
        location: Point<i32, smithay::utils::Logical>,
        activate: bool,
    ) {
        let previous_order: Vec<_> = self.space.elements().cloned().collect();
        let was_mapped = previous_order.iter().any(|mapped| mapped == element);

        self.space.map_element(element.clone(), location, activate);

        if !was_mapped {
            return;
        }

        for mapped in previous_order {
            self.space.raise_element(&mapped, false);
        }
    }

    /// Resize a window to the given rectangle.
    pub fn resize_window(&mut self, window: WindowId, rect: Rect) {
        if let Some(element) = self.find_window(window).cloned() {
            let border_width = self
                .globals()
                .and_then(|g| g.clients.get(&window).map(|c| c.border_width))
                .unwrap_or(0);
            let remap_immediately = self.interactive_motion_active();
            self.set_window_target_location(
                window,
                element.clone(),
                Point::from((rect.x + border_width, rect.y + border_width)),
                remap_immediately,
            );
            if element.toplevel().is_some() {
                let target = (rect.w.max(1), rect.h.max(1));
                let size =
                    smithay::utils::Size::<i32, smithay::utils::Logical>::new(target.0, target.1);
                self.send_toplevel_configure(&element, Some(size));
                self.last_configured_size.insert(window, target);
            }
        }
    }

    /// Raise a window to the top of the stack.
    pub fn raise_window(&mut self, window: WindowId) {
        if let Some(element) = self.find_window(window).cloned() {
            // Focus is handled independently by `set_focus`, so we pass `false`
            self.space.raise_element(&element, false);

            // XWayland requires us to explicitly restack the X11 surface so X clients draw correctly
            if let Some(surface) = element.x11_surface()
                && let Some(xwm) = self.xwm.as_mut()
            {
                let _ = xwm.raise_window(surface);
            }
        }
        self.raise_unmanaged_x11_windows();
    }

    /// Restack windows in the given order.
    pub fn restack(&mut self, windows: &[WindowId]) {
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
