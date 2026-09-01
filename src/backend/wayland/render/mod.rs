//! Wayland compositor rendering.
//!
//! This module contains rendering code for:
//! - Winit (nested) backend
//! - DRM/KMS (standalone) backend
//! - Window borders (shared)

pub mod borders;
pub mod cursor;
pub mod drm;
pub mod frame;
pub mod scene;
pub mod winit;

/// Assemble render elements in front-to-back order from shared scene elements,
/// as required by Smithay's `OutputDamageTracker`.
///
/// Front-to-back order (index 0 = front-most, last index = back-most):
///   1. Emergency shortcut-recovery indicator
///   2. Overlays (dmenu, popups)
///   3. Upper layer shells (Overlay / Top)
///   4. Status bar (top bar and bottom bar)
///   5. Window borders
///   6. Windows and lower layer shells (Bottom / Background)
///
/// Smithay's `OutputDamageTracker::render_output` uses this front-to-back list to
/// perform occlusion culling, then renders elements in reverse (`.rev()`) order
/// back-to-front onto the framebuffer.
macro_rules! assemble_scene_elements {
    ($target:ident, $scene:expr, $space_elements:expr, $num_upper:expr, $suppress_upper:expr, $elements:expr) => {{
        // 1. Compositor safety UI must remain visible over every client.
        for elem in $scene.shortcut_recovery {
            $elements.push($target::Solid(elem));
        }
        // 2. Overlays (dmenu, popups)
        for elem in $scene.overlays {
            $elements.push($target::Surface(elem));
        }
        // 2. Upper layer shells (Overlay / Top)
        let mut space_iter = $space_elements.into_iter();
        for elem in space_iter.by_ref().take($num_upper) {
            if !$suppress_upper {
                $elements.push($target::Space(elem));
            }
        }
        // 3. Status bar (top bar and bottom bar)
        for elem in $scene.bar {
            $elements.push($target::Memory(elem));
        }
        // 4. Borders
        for elem in $scene.borders {
            $elements.push($target::Solid(elem));
        }
        // 5. Windows and lower layer shells (Bottom / Background)
        for elem in space_iter {
            $elements.push($target::Space(elem));
        }
    }};
}
pub(crate) use assemble_scene_elements;

/// Physical top-left corner for a cursor surface tree: `pointer_location` in
/// logical coordinates minus the surface's hotspot, rounded to whole pixels.
fn cursor_surface_loc(
    pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    hotspot: smithay::utils::Point<i32, smithay::utils::Logical>,
) -> smithay::utils::Point<i32, smithay::utils::Physical> {
    smithay::utils::Point::<i32, smithay::utils::Physical>::from((
        (pointer_location.x - hotspot.x as f64).round() as i32,
        (pointer_location.y - hotspot.y as f64).round() as i32,
    ))
}

/// Render a client-provided cursor surface (or DnD icon) tree into render
/// elements, positioned so that `hotspot` sits at `pointer_location`.
///
/// Shared by the DRM and winit backends so cursor and drag-and-drop
/// compositing stays identical. Returns `None` when the surface is dead.
pub fn cursor_surface_render_elements(
    renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    hotspot: smithay::utils::Point<i32, smithay::utils::Logical>,
    scale: f64,
) -> Option<
    Vec<
        smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<
            smithay::backend::renderer::gles::GlesRenderer,
        >,
    >,
> {
    if !smithay::utils::IsAlive::alive(surface) {
        return None;
    }
    let loc = cursor_surface_loc(pointer_location, hotspot);
    Some(
        smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
            renderer,
            surface,
            loc,
            smithay::utils::Scale::from(scale),
            1.0,
            smithay::backend::renderer::element::Kind::Cursor,
        ),
    )
}

#[cfg(test)]
mod cursor_loc_tests {
    use super::cursor_surface_loc;
    use smithay::utils::{Physical, Point};

    #[test]
    fn hotspot_offsets_the_cursor_surface() {
        let loc = cursor_surface_loc(Point::from((100.0, 200.0)), Point::from((4, 12)));
        let expected: Point<i32, Physical> = Point::from((96, 188));
        assert_eq!(loc, expected);
    }

    #[test]
    fn fractional_pointer_positions_round_to_nearest_pixel() {
        let loc = cursor_surface_loc(Point::from((10.4, 11.6)), Point::from((0, 0)));
        let expected: Point<i32, Physical> = Point::from((10, 12));
        assert_eq!(loc, expected);
        // negative half rounds away from zero
        let loc = cursor_surface_loc(Point::from((-1.5, 1.5)), Point::from((0, 0)));
        let expected: Point<i32, Physical> = Point::from((-2, 2));
        assert_eq!(loc, expected);
    }
}
