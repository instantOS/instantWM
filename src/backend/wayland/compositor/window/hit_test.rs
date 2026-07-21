use smithay::desktop::Window;
use smithay::utils::{Logical, Point};

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::compositor::state::WindowIdMarker;
use crate::types::WindowId;

/// Result of a single-pass pointer hit test, resolving both the Wayland
/// surface focus and the WM logical window in one traversal.
pub struct PointerContents {
    /// The Wayland surface that should receive pointer events.
    pub surface: Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<i32, Logical>,
    )>,
    /// The WM-logical window under the pointer (uses outer geometry including
    /// borders, so it can differ from the surface hit).
    pub hovered_win: Option<WindowId>,
}

impl WaylandState {
    /// Single-pass hit test for pointer motion: layers first, then windows.
    ///
    /// Returns both the surface focus and the logical hovered window in one
    /// traversal, avoiding repeated `windows_in_z_order()` allocations.
    pub fn contents_under_pointer(&self, point: Point<f64, Logical>) -> PointerContents {
        // Layer surfaces take priority over all windows.
        if let Some((surface, loc)) = self.layer_surface_under_pointer(point) {
            // Try to resolve a WindowId from the layer surface.
            let hovered_win = self.window_id_from_surface(&surface);
            return PointerContents {
                surface: Some((surface, loc)),
                hovered_win,
            };
        }

        // Single window pass: find both the logical window and surface hit.
        use smithay::desktop::WindowSurfaceType;
        let root = crate::types::Point::from_f64_round(point.x, point.y);
        let (root_x, root_y) = (root.x, root.y);
        let globals = match self.globals() {
            Some(g) => g,
            None => {
                return PointerContents {
                    surface: None,
                    hovered_win: None,
                };
            }
        };

        let mut logical_win: Option<WindowId> = None;
        let mut logical_win_resolved = false;
        let mut surface_hit: Option<(
            smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
            Point<i32, Logical>,
            Option<WindowId>,
        )> = None;

        for (window, typ) in self.windows_in_z_order() {
            let win_id = window.user_data().get::<WindowIdMarker>().map(|m| m.id);

            // Logical hit test (WM geometry including borders).
            if !logical_win_resolved {
                if typ.is_overlay() {
                    if self.overlay_rect_contains(window, root_x, root_y) {
                        logical_win = win_id;
                        logical_win_resolved = true;
                    }
                } else if let Some(win_id) = win_id
                    && let Some(c) = globals.model.client(win_id)
                    && c.total_rect()
                        .contains_point(crate::types::Point::new(root_x, root_y))
                {
                    logical_win = Some(win_id);
                    logical_win_resolved = true;
                }
            }

            // Surface hit test (actual Wayland surface tree).
            //
            // Some XWayland override-redirect overlays (dmenu/rofi-style
            // menus) are deliberately mapped into the Smithay space without a
            // WM WindowId. They still need pointer focus so clicks can be
            // delivered to their surface.
            if surface_hit.is_none()
                && let Some(loc) = self.space.element_location(window)
            {
                let geo_offset = window.geometry().loc;
                let surface_origin = loc - geo_offset;
                if let Some(result) =
                    window.surface_under(point - surface_origin.to_f64(), WindowSurfaceType::ALL)
                {
                    surface_hit = Some((result.0, result.1 + surface_origin, win_id));
                }
            }

            // Both found — no need to continue.
            if logical_win_resolved && surface_hit.is_some() {
                break;
            }
        }

        // If the surface hit belongs to a different window than the logical
        // hit, suppress the surface focus to prevent event fallthrough.
        let surface = match (&logical_win, &surface_hit) {
            (Some(logical), Some((_, _, Some(surface_win)))) if logical != surface_win => None,
            _ => surface_hit.map(|(s, loc, _)| (s, loc)),
        };

        PointerContents {
            surface,
            hovered_win: logical_win,
        }
    }

    /// Resolve a WindowId from a surface via its data map.
    pub(crate) fn window_id_from_surface(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<WindowId> {
        use smithay::wayland::compositor::with_states;
        with_states(surface, |states| {
            states
                .data_map
                .get::<WindowIdMarker>()
                .map(|marker| marker.id)
        })
    }
    /// Get the layer surface under a given point.
    pub fn layer_surface_under_pointer(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<i32, Logical>,
    )> {
        use smithay::desktop::{WindowSurfaceType, layer_map_for_output};

        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        for output in outputs.iter().rev() {
            let Some(output_geo) = self.space.output_geometry(output) else {
                continue;
            };
            let map = layer_map_for_output(output);
            for layer in map.layers().rev() {
                let Some(geo) = map.layer_geometry(layer) else {
                    continue;
                };
                let rel = point - output_geo.loc.to_f64() - geo.loc.to_f64();
                if let Some((surface, loc)) = layer.surface_under(rel, WindowSurfaceType::ALL) {
                    return Some((surface, loc + geo.loc + output_geo.loc));
                }
            }
        }
        None
    }

    /// Get the layer surface that should receive keyboard focus.
    pub fn keyboard_focus_layer_surface(
        &self,
    ) -> Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface> {
        use smithay::desktop::layer_map_for_output;

        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        for output in outputs.iter().rev() {
            let map = layer_map_for_output(output);
            for layer in map.layers().rev() {
                if layer.can_receive_keyboard_focus() {
                    return Some(layer.wl_surface().clone());
                }
            }
        }
        None
    }

    /// Check if the pointer is currently over an overlay, launcher, or unmanaged window.
    pub fn is_pointer_over_overlay(&self, point: Point<f64, Logical>) -> bool {
        let root = crate::types::Point::from_f64_round(point.x, point.y);
        let (root_x, root_y) = (root.x, root.y);
        for (window, typ) in self.windows_in_z_order() {
            if typ.is_overlay() && self.overlay_rect_contains(window, root_x, root_y) {
                return true;
            }
        }
        false
    }

    /// Find the topmost window containing the given logical point within its core geometry.
    /// Used for WM hit-testing to prevent small surfaces from creating focus holes.
    pub fn logical_window_under_pointer(&self, point: Point<f64, Logical>) -> Option<WindowId> {
        let root = crate::types::Point::from_f64_round(point.x, point.y);
        let (root_x, root_y) = (root.x, root.y);
        let globals = self.globals()?;

        for (window, typ) in self.windows_in_z_order() {
            if typ.is_overlay() {
                if self.overlay_rect_contains(window, root_x, root_y) {
                    // We hit an overlay window. Return its WindowId if it has a marker,
                    // otherwise return None to prevent falling through to windows behind.
                    return window.user_data().get::<WindowIdMarker>().map(|m| m.id);
                }
            } else {
                let Some(win_id) = window.user_data().get::<WindowIdMarker>().map(|m| m.id) else {
                    continue;
                };
                if let Some(c) = globals.model.client(win_id)
                    && c.total_rect()
                        .contains_point(crate::types::Point::new(root_x, root_y))
                {
                    return Some(win_id);
                }
            }
        }
        None
    }

    fn overlay_rect_contains(&self, window: &Window, root_x: i32, root_y: i32) -> bool {
        let Some(loc) = self.space.element_location(window) else {
            return false;
        };
        let geo = window.geometry();
        let rel = loc + geo.loc;
        root_x >= rel.x
            && root_x < rel.x + geo.size.w
            && root_y >= rel.y
            && root_y < rel.y + geo.size.h
    }

    /// Get the lock surface under a given point (used when session is locked).
    pub fn lock_surface_under_pointer(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<i32, Logical>,
    )> {
        use smithay::desktop::WindowSurfaceType;

        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        for output in outputs.iter().rev() {
            let Some(output_geo) = self.space.output_geometry(output) else {
                continue;
            };
            if !output_geo.contains(point.to_i32_round()) {
                continue;
            }
            let output_name = output.name();
            if let Some(lock_surface) = self.lock_surfaces.get(&output_name) {
                let rel = point - output_geo.loc.to_f64();
                if let Some((surface, loc)) = smithay::desktop::utils::under_from_surface_tree(
                    lock_surface.wl_surface(),
                    rel,
                    (0, 0),
                    WindowSurfaceType::ALL,
                ) {
                    return Some((surface, loc + output_geo.loc));
                }
            }
        }
        None
    }

    /// Get the surface under a given point.
    pub fn surface_under_pointer(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<i32, Logical>,
    )> {
        use smithay::desktop::WindowSurfaceType;

        for (window, _) in self.windows_in_z_order() {
            let Some(loc) = self.space.element_location(window) else {
                continue;
            };
            let geo_offset = window.geometry().loc;
            let surface_origin = loc - geo_offset;

            // We check ALL surfaces (including children/popups) for all windows
            // in their respective Z-order.
            if let Some(result) =
                window.surface_under(point - surface_origin.to_f64(), WindowSurfaceType::ALL)
            {
                return Some((result.0, result.1 + surface_origin));
            }
        }
        None
    }
}
