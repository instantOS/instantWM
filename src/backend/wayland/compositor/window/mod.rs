use smithay::desktop::Window;
use smithay::utils::{Logical, Point};

use crate::backend::wayland::compositor::WaylandState;
use crate::types::WindowId;

pub mod animations;
pub mod classify;
pub mod focus;
pub mod hit_test;
pub mod lifecycle;
pub mod management;
pub mod properties;
pub mod x11;

pub use classify::WindowType;
pub(crate) use x11::is_unmanaged_x11_overlay;

impl WaylandState {
    /// Check if a window exists.
    pub fn window_exists(&self, window: WindowId) -> bool {
        self.window_index.contains_key(&window)
    }

    /// Allocate a new window ID.
    pub(crate) fn alloc_window_id(&mut self) -> WindowId {
        loop {
            let id = self.next_window_id;
            self.next_window_id = self.next_window_id.wrapping_add(1).max(1);
            let window_id = WindowId::from(id);
            if !self.window_index.contains_key(&window_id) {
                return window_id;
            }
        }
    }

    /// Find a window by ID.
    pub(crate) fn find_window(&self, window: WindowId) -> Option<&Window> {
        self.window_index.get(&window)
    }

    /// Sync client geometry from the compositor's current mapped window state.
    ///
    /// Wayland resizes are configure-driven, so the client may commit a size
    /// smaller than the compositor requested. Keep WM geometry aligned with the
    /// actual mapped element so hover hit-testing and floating restore logic use
    /// the real window bounds.
    pub(crate) fn sync_client_geometry_from_window(&mut self, window: WindowId) {
        // While an animation is in flight the element's space location is the
        // interpolated (or AnimateFrom) position, not the authoritative target.
        // Writing that back into client.geo would corrupt the WM-level geometry
        // (e.g. a floating dialog ends up at the off-screen slide-in origin).
        if self.window_animations.contains_key(&window) {
            return;
        }
        let Some(element) = self.find_window(window).cloned() else {
            return;
        };
        let Some(loc) = self.space.element_location(&element) else {
            return;
        };
        let geo = element.geometry();
        let Some(g) = self.globals_mut() else {
            return;
        };
        let Some(border_width) = g.clients.get(&window).map(|client| client.border_width) else {
            return;
        };

        let rect = crate::types::Rect {
            x: loc.x - border_width,
            y: loc.y - border_width,
            w: geo.size.w.max(1),
            h: geo.size.h.max(1),
        };
        crate::client::sync_client_geometry(g, window, rect);
    }

    /// Request the compositor to warp the pointer to `(x, y)` in logical
    /// screen coordinates.  The warp is deferred until the next event-loop
    /// tick so that the pointer handle and the caller's `pointer_location`
    /// variable can both be updated consistently.
    pub fn request_warp(&mut self, x: f64, y: f64) {
        self.pending_warp = Some(Point::from((x, y)));
    }

    pub(crate) fn begin_interactive_resize(&mut self, window: WindowId) {
        self.active_resizes.insert(window);
    }

    pub(crate) fn end_interactive_resize(&mut self, window: WindowId) {
        self.active_resizes.remove(&window);
        if let Some(element) = self.find_window(window).cloned() {
            self.send_toplevel_configure(&element, None);
        }
    }

    /// Consume and return the pending warp target, if any.
    pub fn take_pending_warp(&mut self) -> Option<Point<f64, Logical>> {
        self.pending_warp.take()
    }

    pub(crate) fn raise_unmanaged_x11_windows(&mut self) {
        let overlays: Vec<_> = self
            .windows_in_z_order()
            .into_iter()
            .filter(|(_, typ)| matches!(typ, WindowType::Launcher | WindowType::Overlay))
            .map(|(w, _)| w.clone())
            .collect();
        for w in overlays {
            self.space.raise_element(&w, false);
        }
    }

    /// Collect all overlay/unmanaged windows (dmenu, override-redirect popups,
    /// etc.) that should be rendered above the bar but below the cursor.
    ///
    /// Returns `(window, physical_location)` pairs ready for `AsRenderElements`.
    pub fn overlay_windows_for_render(
        &self,
        x_offset: i32,
    ) -> Vec<(Window, Point<i32, smithay::utils::Physical>)> {
        use smithay::utils::Physical;

        self.windows_in_z_order()
            .into_iter()
            .filter(|(_, typ)| matches!(typ, WindowType::Launcher | WindowType::Overlay))
            .filter_map(|(w, _)| {
                let loc = self.space.element_location(w)?;
                // Translate from global compositor coordinates to the
                // per-output local coordinate space, then convert to physical
                // pixels.
                let phys = Point::<i32, Physical>::from((loc.x - x_offset, loc.y));
                Some((w.clone(), phys))
            })
            .collect()
    }
}
