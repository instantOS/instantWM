use smithay::utils::{Logical, Point};

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::compositor::state::WindowIdMarker;
use crate::types::WindowId;

use super::classify::WindowType;

impl WaylandState {
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

    /// Find the topmost window containing the given logical point within its core geometry.
    /// Used for WM hit-testing to prevent small surfaces from creating focus holes.
    pub fn logical_window_under_pointer(&self, point: Point<f64, Logical>) -> Option<WindowId> {
        let root_x = point.x.round() as i32;
        let root_y = point.y.round() as i32;
        let globals = self.globals()?;

        for (window, typ) in self.windows_in_z_order() {
            let Some(win_id) = window.user_data().get::<WindowIdMarker>().map(|m| m.id) else {
                continue;
            };

            if matches!(
                typ,
                WindowType::Launcher | WindowType::Overlay | WindowType::Unmanaged
            ) {
                let Some(loc) = self.space.element_location(window) else {
                    continue;
                };
                let geo = window.geometry();
                let relative_loc = loc + geo.loc;

                if root_x >= relative_loc.x
                    && root_x < relative_loc.x + geo.size.w
                    && root_y >= relative_loc.y
                    && root_y < relative_loc.y + geo.size.h
                {
                    return Some(win_id);
                }
            } else {
                // Fall back to managed windows with borders
                if let Some(c) = globals.clients.get(&win_id) {
                    let bw = c.border_width;
                    // c.geo x/y are outer coordinates, so the total width spans c.geo.w + 2*bw
                    if root_x >= c.geo.x
                        && root_x < c.geo.x + c.geo.w + 2 * bw
                        && root_y >= c.geo.y
                        && root_y < c.geo.y + c.geo.h + 2 * bw
                    {
                        return Some(win_id);
                    }
                }
            }
        }
        None
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
