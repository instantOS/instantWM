use smithay::desktop::Window;
use smithay::utils::{Logical, Point};

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::compositor::state::WindowIdMarker;
use crate::types::WindowId;

pub(crate) type SurfaceFocus = (
    smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    Point<i32, Logical>,
);

/// Result of a single-pass pointer hit test, resolving both the Wayland
/// surface focus and the WM logical window in one traversal.
pub(crate) struct PointerContents {
    /// The Wayland surface that should receive pointer events.
    pub(crate) surface: Option<SurfaceFocus>,
    /// The WM-logical window under the pointer (uses outer geometry including
    /// borders, so it can differ from the surface hit).
    pub(crate) hovered_win: Option<WindowId>,
}

struct RankedSurfaceHit {
    focus: SurfaceFocus,
    window: Option<WindowId>,
    rank: usize,
}

/// Decide whether the actual surface hit is visually above the logical WM hit.
///
/// A popup may extend outside its owner's WM rectangle and cover a window
/// below it. Conversely, a lower window's client surface may be visible to
/// surface-tree hit testing through the compositor-drawn decoration of a
/// higher window. The candidate encountered first in compositor z-order wins.
fn surface_hit_takes_precedence(logical_rank: Option<usize>, surface_rank: usize) -> bool {
    logical_rank.is_none_or(|logical_rank| surface_rank <= logical_rank)
}

impl WaylandState {
    /// Single-pass hit test for pointer motion: layers first, then windows.
    ///
    /// Returns both the surface focus and the logical hovered window in one
    /// traversal, avoiding repeated `windows_in_z_order()` allocations.
    pub(crate) fn contents_under_pointer(&self, point: Point<f64, Logical>) -> PointerContents {
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
        let mut logical_rank: Option<usize> = None;
        let mut logical_win_resolved = false;
        let mut surface_hit: Option<RankedSurfaceHit> = None;

        for (rank, (window, typ)) in self.windows_in_z_order().into_iter().enumerate() {
            let win_id = window.user_data().get::<WindowIdMarker>().map(|m| m.id);

            // Logical hit test (WM geometry including borders).
            if !logical_win_resolved {
                if typ.is_overlay() {
                    if self.overlay_rect_contains(window, root_x, root_y) {
                        logical_win = win_id;
                        logical_rank = Some(rank);
                        logical_win_resolved = true;
                    }
                } else if let Some(win_id) = win_id
                    && let Some(c) = globals.model.client(win_id)
                    && c.total_rect()
                        .contains_point(crate::types::Point::new(root_x, root_y))
                {
                    logical_win = Some(win_id);
                    logical_rank = Some(rank);
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
                    surface_hit = Some(RankedSurfaceHit {
                        focus: (result.0, result.1 + surface_origin),
                        window: win_id,
                        rank,
                    });
                }
            }

            // Both found — no need to continue.
            if logical_win_resolved && surface_hit.is_some() {
                break;
            }
        }

        // Reconcile the two coordinate models using compositor z-order. A
        // popup found before the logical rectangle below it wins; a client
        // surface found behind a higher window's decorations is suppressed.
        // Unmanaged overlays have no WindowId and deliberately suppress hover
        // focus behind them when they are the higher candidate.
        let (surface, hovered_win) = match surface_hit {
            Some(hit) if surface_hit_takes_precedence(logical_rank, hit.rank) => {
                (Some(hit.focus), hit.window)
            }
            _ => (None, logical_win),
        };

        PointerContents {
            surface,
            hovered_win,
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
    pub(crate) fn layer_surface_under_pointer(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<SurfaceFocus> {
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

    /// Check if the pointer is currently over an overlay, launcher, or unmanaged window.
    pub(crate) fn is_pointer_over_overlay(&self, point: Point<f64, Logical>) -> bool {
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
    pub(crate) fn logical_window_under_pointer(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<WindowId> {
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
    pub(crate) fn lock_surface_under_pointer(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<SurfaceFocus> {
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
}

#[cfg(test)]
mod tests {
    use super::surface_hit_takes_precedence;

    #[test]
    fn popup_above_underlying_logical_window_takes_precedence() {
        assert!(surface_hit_takes_precedence(Some(1), 0));
    }

    #[test]
    fn surface_behind_higher_decoration_is_suppressed() {
        assert!(!surface_hit_takes_precedence(Some(0), 1));
    }

    #[test]
    fn surface_wins_when_no_logical_window_was_hit() {
        assert!(surface_hit_takes_precedence(None, 3));
    }

    #[test]
    fn surface_and_logical_hit_from_same_window_stay_focused() {
        assert!(surface_hit_takes_precedence(Some(2), 2));
    }
}
