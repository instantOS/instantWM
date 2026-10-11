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
pub mod mirror;
pub mod scene;
pub mod winit;

/// Assemble the shared scene bands in Smithay's front-to-back order.
macro_rules! assemble_scene_elements {
    ($target:ident, $scene:expr, $windows:expr, $elements:expr) => {{
        use crate::backend::wayland::stacking::SceneBand;
        let mut scene = $scene;
        let mut windows = $windows;
        for band in SceneBand::FRONT_TO_BACK {
            match band {
                SceneBand::SafetyUi => $elements.extend(
                    std::mem::take(&mut scene.shortcut_recovery)
                        .into_iter()
                        .map($target::Solid),
                ),
                SceneBand::OverlayLayer => $elements.extend(
                    std::mem::take(&mut scene.layer_overlay)
                        .into_iter()
                        .map($target::Surface),
                ),
                SceneBand::WindowOverlay => $elements.extend(
                    std::mem::take(&mut scene.overlays)
                        .into_iter()
                        .map($target::Surface),
                ),
                SceneBand::TopLayer => $elements.extend(
                    std::mem::take(&mut scene.layer_top)
                        .into_iter()
                        .map($target::Surface),
                ),
                SceneBand::Bar => $elements.extend(
                    std::mem::take(&mut scene.bar)
                        .into_iter()
                        .map($target::Memory),
                ),
                SceneBand::Borders => $elements.extend(
                    std::mem::take(&mut scene.borders)
                        .into_iter()
                        .map($target::Solid),
                ),
                SceneBand::Windows => $elements.extend(
                    std::mem::take(&mut windows)
                        .into_iter()
                        .map($target::Surface),
                ),
                SceneBand::LowerLayers => $elements.extend(
                    std::mem::take(&mut scene.layer_lower)
                        .into_iter()
                        .map($target::Surface),
                ),
            }
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

#[cfg(test)]
mod scene_order_tests {
    // Exercise the actual assembler used by both renderers without a GPU.
    // Labels stand for already-built render elements from each scene band.
    struct Scene {
        shortcut_recovery: Vec<&'static str>,
        layer_overlay: Vec<&'static str>,
        overlays: Vec<&'static str>,
        layer_top: Vec<&'static str>,
        bar: Vec<&'static str>,
        borders: Vec<&'static str>,
        layer_lower: Vec<&'static str>,
    }

    #[derive(Debug, PartialEq)]
    enum Element {
        Surface(&'static str),
        Solid(&'static str),
        Memory(&'static str),
    }

    #[test]
    fn selector_covers_application_menus_while_windows_cover_lower_shells() {
        let scene = Scene {
            shortcut_recovery: vec!["recovery"],
            layer_overlay: vec!["selector"],
            overlays: vec!["menu"],
            layer_top: vec!["panel"],
            bar: vec!["bar"],
            borders: vec!["border"],
            layer_lower: vec!["desktop-widget", "wallpaper"],
        };
        let mut elements = Vec::new();
        assemble_scene_elements!(Element, scene, vec!["window"], elements);
        assert_eq!(
            elements,
            vec![
                Element::Solid("recovery"),
                Element::Surface("selector"),
                Element::Surface("menu"),
                Element::Surface("panel"),
                Element::Memory("bar"),
                Element::Solid("border"),
                Element::Surface("window"),
                Element::Surface("desktop-widget"),
                Element::Surface("wallpaper"),
            ]
        );
    }

    #[test]
    fn fullscreen_scene_keeps_selector_when_top_and_bar_are_hidden() {
        let scene = Scene {
            shortcut_recovery: vec![],
            layer_overlay: vec!["selector"],
            overlays: vec![],
            layer_top: vec![],
            bar: vec![],
            borders: vec![],
            layer_lower: vec![],
        };
        let mut elements = Vec::new();
        assemble_scene_elements!(Element, scene, vec!["fullscreen"], elements);
        assert_eq!(
            elements,
            vec![Element::Surface("selector"), Element::Surface("fullscreen")]
        );
    }
}
