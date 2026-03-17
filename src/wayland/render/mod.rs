//! Wayland compositor rendering.
//!
//! This module contains rendering code for:
//! - Winit (nested) backend
//! - DRM/KMS (standalone) backend
//! - Window borders (shared)

pub mod borders;
pub mod drm;
pub mod winit;

/// Assemble render elements in z-order from shared scene elements.
///
/// Both the DRM and winit backends use the same layering order:
///   1. Overlays (dmenu, popups)
///   2. Upper layer shells (Overlay / Top)
///   3. Status bar
///   4. Borders
///   5. Windows and lower layer shells (Bottom / Background)
///
/// The only difference is the concrete render-element enum (`DrmExtras` vs
/// `WaylandExtras`), so this macro generates the assembly for any target
/// type that has `Surface`, `Space`, `Memory`, and `Solid` variants.
macro_rules! assemble_scene_elements {
    ($target:ident, $scene:expr, $space_elements:expr, $num_upper:expr, $elements:expr) => {{
        // 1. Overlays (dmenu, popups)
        for elem in $scene.overlays {
            $elements.push($target::Surface(elem));
        }
        // 2. Upper layer shells (Overlay / Top)
        let mut space_iter = $space_elements.into_iter();
        for elem in space_iter.by_ref().take($num_upper) {
            $elements.push($target::Space(elem));
        }
        // 3. Status Bar
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
