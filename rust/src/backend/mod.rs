//! Backend abstraction.
//!
//! This module supports multiple window-system backends:
//! - **X11** (always available) — the original `x11rb`-based backend.
//! - **Wayland** (feature-gated behind `wayland_backend`) — a Smithay-based
//!   Wayland compositor backend.

pub mod wayland;
pub mod x11;

use crate::backend::wayland::WaylandBackend;
use crate::backend::x11::{X11Backend, X11BackendRef};
use crate::types::{Rect, WindowId};

/// Core backend operations required by the WM.
pub trait BackendOps {
    fn resize_window(&self, window: WindowId, rect: Rect);
    fn raise_window(&self, window: WindowId);
    fn restack(&self, windows: &[WindowId]);
    fn set_focus(&self, window: WindowId);
    fn map_window(&self, window: WindowId);
    fn unmap_window(&self, window: WindowId);
    fn set_border_width(&self, window: WindowId, width: i32);
    fn window_exists(&self, window: WindowId) -> bool;
    fn flush(&self);
    /// Read the window title from the backend.
    ///
    /// Returns `None` when the title is not available or the backend
    /// does not track titles (e.g. X11 titles are read separately via
    /// X properties).
    fn window_title(&self, _window: WindowId) -> Option<String> {
        None
    }

    /// Switch keyboard layout
    fn set_keyboard_layout(
        &self,
        _layout: &str,
        _variant: &str,
        _options: Option<&str>,
        _model: Option<&str>,
    ) {
    }

    fn list_displays(&self) -> Vec<String> {
        vec![]
    }
    fn list_display_modes(&self, _display: &str) -> Vec<String> {
        vec![]
    }
    fn set_display_mode(&self, _display: &str, _width: i32, _height: i32) {}
}

/// Owned backend implementation.
pub enum Backend {
    X11(X11Backend),
    Wayland(WaylandBackend),
}

impl Backend {
    pub fn x11(&self) -> Option<&X11Backend> {
        match self {
            Self::X11(x11) => Some(x11),
            Self::Wayland(_) => None,
        }
    }

    pub fn x11_mut(&mut self) -> Option<&mut X11Backend> {
        match self {
            Self::X11(x11) => Some(x11),
            Self::Wayland(_) => None,
        }
    }

    /// Create a borrowed reference to delegate operations to.
    fn as_ref(&self) -> BackendRef<'_> {
        BackendRef::from_backend(self)
    }
}

impl BackendOps for Backend {
    fn resize_window(&self, window: WindowId, rect: Rect) {
        self.as_ref().resize_window(window, rect)
    }

    fn raise_window(&self, window: WindowId) {
        self.as_ref().raise_window(window)
    }

    fn restack(&self, windows: &[WindowId]) {
        self.as_ref().restack(windows)
    }

    fn set_focus(&self, window: WindowId) {
        self.as_ref().set_focus(window)
    }

    fn map_window(&self, window: WindowId) {
        self.as_ref().map_window(window)
    }

    fn unmap_window(&self, window: WindowId) {
        self.as_ref().unmap_window(window)
    }

    fn set_border_width(&self, window: WindowId, width: i32) {
        self.as_ref().set_border_width(window, width)
    }

    fn window_exists(&self, window: WindowId) -> bool {
        self.as_ref().window_exists(window)
    }

    fn flush(&self) {
        self.as_ref().flush()
    }

    fn window_title(&self, window: WindowId) -> Option<String> {
        self.as_ref().window_title(window)
    }

    fn set_keyboard_layout(
        &self,
        layout: &str,
        variant: &str,
        options: Option<&str>,
        model: Option<&str>,
    ) {
        self.as_ref()
            .set_keyboard_layout(layout, variant, options, model)
    }

    fn list_displays(&self) -> Vec<String> {
        self.as_ref().list_displays()
    }

    fn list_display_modes(&self, display: &str) -> Vec<String> {
        self.as_ref().list_display_modes(display)
    }

    fn set_display_mode(&self, display: &str, width: i32, height: i32) {
        self.as_ref().set_display_mode(display, width, height)
    }
}

/// Borrowed backend view for context wiring.
pub enum BackendRef<'a> {
    X11(X11BackendRef<'a>),
    Wayland(&'a WaylandBackend),
}

impl<'a> BackendRef<'a> {
    pub fn from_backend(backend: &'a Backend) -> Self {
        match backend {
            Backend::X11(x11) => BackendRef::X11(X11BackendRef::new(&x11.conn, x11.screen_num)),
            Backend::Wayland(wayland) => BackendRef::Wayland(wayland),
        }
    }

    pub fn from_x11(conn: &'a x11rb::rust_connection::RustConnection, screen_num: usize) -> Self {
        BackendRef::X11(X11BackendRef::new(conn, screen_num))
    }

    pub fn x11_conn(&self) -> Option<(&'a x11rb::rust_connection::RustConnection, usize)> {
        match self {
            BackendRef::X11(x11) => Some((x11.conn, x11.screen_num)),
            BackendRef::Wayland(_) => None,
        }
    }

    pub fn reborrow(&self) -> BackendRef<'_> {
        match self {
            BackendRef::X11(x11) => BackendRef::X11(X11BackendRef::new(x11.conn, x11.screen_num)),
            BackendRef::Wayland(wayland) => BackendRef::Wayland(wayland),
        }
    }
}

impl BackendOps for BackendRef<'_> {
    fn resize_window(&self, window: WindowId, rect: Rect) {
        match self {
            BackendRef::X11(x11) => x11.resize_window(window, rect),
            BackendRef::Wayland(wayland) => wayland.resize_window(window, rect),
        }
    }

    fn raise_window(&self, window: WindowId) {
        match self {
            BackendRef::X11(x11) => x11.raise_window(window),
            BackendRef::Wayland(wayland) => wayland.raise_window(window),
        }
    }

    fn restack(&self, windows: &[WindowId]) {
        match self {
            BackendRef::X11(x11) => x11.restack(windows),
            BackendRef::Wayland(wayland) => wayland.restack(windows),
        }
    }

    fn set_focus(&self, window: WindowId) {
        match self {
            BackendRef::X11(x11) => x11.set_focus(window),
            BackendRef::Wayland(wayland) => wayland.set_focus(window),
        }
    }

    fn map_window(&self, window: WindowId) {
        match self {
            BackendRef::X11(x11) => x11.map_window(window),
            BackendRef::Wayland(wayland) => wayland.map_window(window),
        }
    }

    fn unmap_window(&self, window: WindowId) {
        match self {
            BackendRef::X11(x11) => x11.unmap_window(window),
            BackendRef::Wayland(wayland) => wayland.unmap_window(window),
        }
    }

    fn set_border_width(&self, window: WindowId, width: i32) {
        match self {
            BackendRef::X11(x11) => x11.set_border_width(window, width),
            BackendRef::Wayland(wayland) => wayland.set_border_width(window, width),
        }
    }

    fn window_exists(&self, window: WindowId) -> bool {
        match self {
            BackendRef::X11(x11) => x11.window_exists(window),
            BackendRef::Wayland(wayland) => wayland.window_exists(window),
        }
    }

    fn flush(&self) {
        match self {
            BackendRef::X11(x11) => x11.flush(),
            BackendRef::Wayland(wayland) => wayland.flush(),
        }
    }

    fn window_title(&self, window: WindowId) -> Option<String> {
        match self {
            BackendRef::X11(x11) => x11.window_title(window),
            BackendRef::Wayland(wayland) => wayland.window_title(window),
        }
    }

    fn set_keyboard_layout(
        &self,
        layout: &str,
        variant: &str,
        options: Option<&str>,
        model: Option<&str>,
    ) {
        match self {
            BackendRef::X11(x11) => x11.set_keyboard_layout(layout, variant, options, model),
            BackendRef::Wayland(wayland) => {
                wayland.set_keyboard_layout(layout, variant, options, model)
            }
        }
    }

    fn list_displays(&self) -> Vec<String> {
        match self {
            BackendRef::X11(x11) => x11.list_displays(),
            BackendRef::Wayland(wayland) => wayland.list_displays(),
        }
    }

    fn list_display_modes(&self, display: &str) -> Vec<String> {
        match self {
            BackendRef::X11(x11) => x11.list_display_modes(display),
            BackendRef::Wayland(wayland) => wayland.list_display_modes(display),
        }
    }

    fn set_display_mode(&self, display: &str, width: i32, height: i32) {
        match self {
            BackendRef::X11(x11) => x11.set_display_mode(display, width, height),
            BackendRef::Wayland(wayland) => wayland.set_display_mode(display, width, height),
        }
    }
}
