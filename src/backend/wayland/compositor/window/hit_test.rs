use smithay::utils::{Logical, Point};

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::compositor::state::WindowIdMarker;
use crate::types::WindowId;

use super::classify::WindowType;

/// Result of a unified hit-test query.
#[derive(Debug, Default, Clone)]
pub struct HitTestResult {
    /// Layer surface under pointer (if any).
    pub layer_surface: Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<i32, Logical>,
    )>,
    /// Regular window surface under pointer (if any).
    pub window_surface: Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<i32, Logical>,
    )>,
    /// Logical window ID under pointer (if any).
    pub hovered_win: Option<WindowId>,
}

impl WaylandState {
    /// Unified hit-test: find all surfaces/windows under pointer in a single pass.
    ///
    /// This is more efficient than separate hit-test calls, as it iterates through
    /// outputs, layers, and windows only once.
    ///
    /// Returns a `HitTestResult` containing:
    /// - `layer_surface`: topmost layer surface (panel, overlay, etc.)
    /// - `window_surface`: topmost regular window surface (including popups/subsurfaces)
    /// - `hovered_win`: logical window ID containing the point
    pub fn hit_test(&self, point: Point<f64, Logical>) -> HitTestResult {
        use smithay::desktop::{WindowSurfaceType, layer_map_for_output};

        let mut result = HitTestResult::default();
        let root_x = point.x.round() as i32;
        let root_y = point.y.round() as i32;
        let globals = &self.wm.g;

        // Phase 1: Check layer surfaces (panels, overlays, etc.)
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
                    result.layer_surface = Some((surface, loc + geo.loc + output_geo.loc));
                    // Layer surfaces are topmost, no need to check other layers
                    break;
                }
            }
            // If we found a layer surface, don't check windows below
            if result.layer_surface.is_some() {
                break;
            }
        }

        // Phase 2: Check regular windows (only if no layer surface found)
        if result.layer_surface.is_none() {
            for (window, typ) in self.windows_in_z_order() {
                let Some(win_id) = window.user_data().get::<WindowIdMarker>().map(|m| m.id) else {
                    continue;
                };

                // Check for logical window containment
                if matches!(
                    typ,
                    WindowType::Launcher | WindowType::Overlay | WindowType::Unmanaged
                ) {
                    let Some(loc) = self.space.element_location(window) else {
                        continue;
                    };
                    let geo = window.geometry();
                    let relative_loc = loc + geo.loc;

                    if result.hovered_win.is_none()
                        && root_x >= relative_loc.x
                        && root_x < relative_loc.x + geo.size.w
                        && root_y >= relative_loc.y
                        && root_y < relative_loc.y + geo.size.h
                    {
                        result.hovered_win = Some(win_id);
                    }
                } else {
                    // Managed windows with borders
                    if result.hovered_win.is_none()
                        && let Some(c) = globals.clients.get(&win_id)
                    {
                        let bw = c.border_width;
                        if root_x >= c.geo.x
                            && root_x < c.geo.x + c.geo.w + 2 * bw
                            && root_y >= c.geo.y
                            && root_y < c.geo.y + c.geo.h + 2 * bw
                        {
                            result.hovered_win = Some(win_id);
                        }
                    }
                }

                // Check for surface hit (for pointer events)
                if result.window_surface.is_none() {
                    let Some(loc) = self.space.element_location(window) else {
                        continue;
                    };
                    let geo_offset = window.geometry().loc;
                    let surface_origin = loc - geo_offset;

                    if let Some(surface_result) = window
                        .surface_under(point - surface_origin.to_f64(), WindowSurfaceType::ALL)
                    {
                        result.window_surface =
                            Some((surface_result.0, surface_result.1 + surface_origin));
                    }
                }

                // Early exit if we found both
                if result.window_surface.is_some() && result.hovered_win.is_some() {
                    break;
                }
            }
        }

        result
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
}
