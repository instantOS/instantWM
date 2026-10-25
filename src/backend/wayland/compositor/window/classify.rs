use crate::backend::wayland::compositor::WaylandNativeState;
use smithay::desktop::Window;
use smithay::utils::IsAlive;

use crate::backend::wayland::compositor::state::WindowIdMarker;

// ---------------------------------------------------------------------------
// Window Type Classification
// ---------------------------------------------------------------------------

/// Classification of a window's type for focus and input routing decisions.
///
/// This unified classifier replaces the scattered overlay detection logic
/// and provides a single source of truth for window categorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowType {
    /// Normal tiled or floating window - receives focus normally
    Normal,
    /// Overlay window (dmenu, popup, menu) with elevated stacking/focus behavior
    Overlay,
    /// Launcher window (dmenu, instantmenu) - special focus behavior
    Launcher,
    /// Unmanaged X11 override-redirect window
    Unmanaged,
    /// Window that is dying or dead - should not receive focus
    Dying,
}

impl WindowType {
    /// Check if this type of window is an overlay type (Launcher, Overlay, or Unmanaged).
    pub fn is_overlay(self) -> bool {
        matches!(
            self,
            WindowType::Launcher | WindowType::Overlay | WindowType::Unmanaged
        )
    }
}

impl WaylandNativeState {
    /// Owned snapshot of the current input/render ordering.
    ///
    /// Pointer motion can hit-test both its old and proposed positions against
    /// one snapshot without retaining an immutable borrow of `WaylandState`
    /// across Smithay pointer dispatch.
    pub(crate) fn pointer_hit_snapshot(&self) -> Vec<(Window, WindowType)> {
        #[cfg(test)]
        super::hit_test::record_pointer_hit_snapshot();

        collect_in_scene_order(
            self.space
                .elements()
                .rev()
                .map(|window| (window.clone(), self.classify_window(window))),
        )
    }
}

impl WaylandNativeState {
    /// Classify a window's type for focus and input routing decisions.
    ///
    /// This is the single source of truth for window classification.
    /// All focus decisions should use this method instead of ad-hoc checks.
    pub fn classify_window(&self, window: &Window) -> WindowType {
        // Check if window is dying first - this takes precedence
        if !window.alive() {
            return WindowType::Dying;
        }

        // Inspect launcher metadata once. It takes precedence over both
        // override-redirect and the map-time overlay marker.
        if let Some(x11) = window.x11_surface() {
            if super::x11::is_launcher_x11_surface(x11) {
                return WindowType::Launcher;
            }
            if x11.is_override_redirect() {
                return WindowType::Unmanaged;
            }
            if super::x11::is_overlay_window_type(x11.window_type()) {
                return WindowType::Overlay;
            }
        }
        if window
            .user_data()
            .get::<WindowIdMarker>()
            .is_some_and(|marker| marker.is_overlay)
        {
            WindowType::Overlay
        } else {
            WindowType::Normal
        }
    }

    /// Iterator over windows in z-order (top-to-bottom), along with their type.
    ///
    /// This follows the render-order defined in `assemble_scene_elements!`:
    /// 1. Overlays and Launchers
    /// 2. Others (Normal, Unmanaged)
    pub fn windows_in_z_order(&self) -> Vec<(&Window, WindowType)> {
        collect_in_scene_order(
            self.space
                .elements()
                .rev()
                .map(|window| (window, self.classify_window(window))),
        )
    }
}

/// Preserve Space order within each foreground/ordinary group. Borrowed render
/// views and owned input snapshots each collect directly into one vector.
fn collect_in_scene_order<T>(
    windows: impl Iterator<Item = (T, WindowType)>,
) -> Vec<(T, WindowType)> {
    let mut windows: Vec<_> = windows.collect();
    windows.sort_by_key(|(_, typ)| !typ.is_overlay());
    windows
}

#[cfg(test)]
mod tests {
    use super::{WindowType, collect_in_scene_order};

    #[test]
    fn foreground_partition_preserves_stacking_within_both_groups() {
        use WindowType::*;
        let windows = [
            (1, Normal),
            (2, Overlay),
            (3, Unmanaged),
            (4, Normal),
            (5, Launcher),
            (6, Dying),
        ];
        let ordered = collect_in_scene_order(windows.into_iter());
        assert_eq!(
            ordered,
            [
                (2, Overlay),
                (3, Unmanaged),
                (5, Launcher),
                (1, Normal),
                (4, Normal),
                (6, Dying)
            ]
        );
    }
}
