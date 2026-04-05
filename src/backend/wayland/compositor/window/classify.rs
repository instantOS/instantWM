use smithay::desktop::Window;
use smithay::utils::IsAlive;

use crate::backend::wayland::compositor::WaylandState;
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
    /// Overlay window (dmenu, popup, menu) - focus suppresses WM shortcuts
    Overlay,
    /// Launcher window (dmenu, instantmenu) - special focus behavior
    Launcher,
    /// Unmanaged X11 override-redirect window
    Unmanaged,
    /// Window that is dying or dead - should not receive focus
    Dying,
}

impl WaylandState {
    /// Classify a window's type for focus and input routing decisions.
    ///
    /// This is the single source of truth for window classification.
    /// All focus decisions should use this method instead of ad-hoc checks.
    pub fn classify_window(&self, window: &Window) -> WindowType {
        // Check if window is dying first - this takes precedence
        if !window.alive() {
            return WindowType::Dying;
        }

        // Check for unmanaged X11 overlay
        if let Some(x11) = window.x11_surface()
            && super::x11::is_unmanaged_x11_overlay(x11)
        {
            if super::x11::is_launcher_x11_surface(x11) {
                return WindowType::Launcher;
            }
            if x11.is_override_redirect() {
                return WindowType::Unmanaged;
            }
            return WindowType::Overlay;
        }

        // Check window marker for overlay classification
        if let Some(marker) = window.user_data().get::<WindowIdMarker>()
            && marker.is_overlay
        {
            // Check if it's a launcher by title/class
            if let Some(x11) = window.x11_surface()
                && super::x11::is_launcher_x11_surface(x11)
            {
                return WindowType::Launcher;
            }
            return WindowType::Overlay;
        }

        // Check X11 surface properties
        if let Some(x11) = window.x11_surface()
            && super::x11::is_launcher_x11_surface(x11)
        {
            return WindowType::Launcher;
        }

        WindowType::Normal
    }

    /// Check if a window should suppress WM keyboard shortcuts when focused.
    ///
    /// Returns true for overlay windows (dmenu, popups, menus) where
    /// keyboard input should go to the window without triggering keybindings.
    pub fn should_suppress_shortcuts_for(&self, window: &Window) -> bool {
        match self.classify_window(window) {
            WindowType::Overlay | WindowType::Launcher | WindowType::Unmanaged => true,
            WindowType::Normal | WindowType::Dying => false,
        }
    }

    /// Iterator over windows in z-order (top-to-bottom), along with their type.
    ///
    /// This follows the render-order defined in `assemble_scene_elements!`:
    /// 1. Overlays and Launchers
    /// 2. Others (Normal, Unmanaged)
    pub fn windows_in_z_order(&self) -> Vec<(&Window, WindowType)> {
        let mut windows: Vec<(&Window, WindowType)> = self
            .space
            .elements()
            .rev()
            .map(|w| (w, self.classify_window(w)))
            .collect();

        windows.sort_by_key(|(_, typ)| match typ {
            WindowType::Launcher | WindowType::Overlay | WindowType::Unmanaged => 0,
            _ => 1,
        });
        windows
    }
}
