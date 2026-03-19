//! Backend abstraction.
//!
//! This module supports multiple window-system backends:
//! - **X11** (always available) — the original `x11rb`-based backend.
//! - **Wayland** (feature-gated behind `wayland_backend`) — a Smithay-based
//!   Wayland compositor backend.

pub mod wayland;
pub mod x11;

use crate::backend::wayland::WaylandBackend;
use crate::backend::x11::{X11BackendRef, X11RuntimeConfig};
use crate::types::{Rect, Systray, WaylandSystray, WaylandSystrayMenu, WindowId};

#[derive(Debug, Clone)]
pub struct BackendOutputInfo {
    pub name: String,
    pub rect: Rect,
}

/// Core backend operations required by the WM.
pub trait BackendOps {
    fn resize_window(&self, window: WindowId, rect: Rect);
    fn raise_window(&self, window: WindowId);
    fn restack(&self, windows: &[WindowId]);
    fn set_focus(&self, window: WindowId);
    fn map_window(&self, window: WindowId);
    fn unmap_window(&self, window: WindowId);

    /// Check if a window still exists in the backend.
    ///
    /// Returns `true` if the window exists, `false` otherwise.
    /// This is a query method that returns state rather than performing an action.
    fn window_exists(&self, window: WindowId) -> bool;
    fn flush(&self);

    /// Get current pointer location in root coordinates.
    ///
    /// Returns `None` if the pointer position cannot be determined
    /// (e.g., no pointer device available).
    fn pointer_location(&self) -> Option<(i32, i32)>;

    /// Warp pointer to (x, y) in root coordinates.
    fn warp_pointer(&self, x: f64, y: f64);

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

    /// Set monitor configuration
    fn set_monitor_config(&self, _name: &str, _config: &crate::config::config_toml::MonitorConfig) {
    }

    /// Get current outputs from the backend
    fn get_outputs(&self) -> Vec<BackendOutputInfo> {
        Vec::new()
    }
}

/// X11-specific backend data.
pub struct X11BackendData {
    pub conn: x11rb::rust_connection::RustConnection,
    pub screen_num: usize,
    pub x11_runtime: X11RuntimeConfig,
    pub systray: Option<Systray>,
}

/// Wayland-specific backend data.
pub struct WaylandBackendData {
    pub backend: WaylandBackend,
    pub bar_painter: crate::bar::wayland::WaylandBarPainter,
    pub wayland_systray: WaylandSystray,
    pub wayland_systray_menu: Option<WaylandSystrayMenu>,
    pub wayland_systray_runtime: Option<crate::systray::wayland::WaylandSystrayRuntime>,
}

/// Owned backend implementation.
///
/// Each variant owns the backend-specific connection **and** runtime state
/// (atoms, cursors, systray, drawing helpers, etc.) so that `Wm` stays
/// backend-agnostic at the type level.
pub enum Backend {
    X11(Box<X11BackendData>),
    Wayland(Box<WaylandBackendData>),
}

impl Backend {
    pub fn new_x11(conn: x11rb::rust_connection::RustConnection, screen_num: usize) -> Self {
        Self::X11(Box::new(X11BackendData {
            conn,
            screen_num,
            x11_runtime: X11RuntimeConfig::default(),
            systray: None,
        }))
    }

    pub fn new_wayland(backend: WaylandBackend) -> Self {
        Self::Wayland(Box::new(WaylandBackendData {
            backend,
            bar_painter: crate::bar::wayland::WaylandBarPainter::default(),
            wayland_systray: WaylandSystray::default(),
            wayland_systray_menu: None,
            wayland_systray_runtime: None,
        }))
    }

    /// Shorthand: get the X11 connection + screen, if running X11.
    pub fn x11_conn(&self) -> Option<(&x11rb::rust_connection::RustConnection, usize)> {
        match self {
            Self::X11(data) => Some((&data.conn, data.screen_num)),
            Self::Wayland(_) => None,
        }
    }

    pub fn x11_conn_mut(&mut self) -> Option<(&mut x11rb::rust_connection::RustConnection, usize)> {
        match self {
            Self::X11(data) => Some((&mut data.conn, data.screen_num)),
            Self::Wayland(_) => None,
        }
    }

    pub fn x11_data(&self) -> Option<&X11BackendData> {
        match self {
            Self::X11(data) => Some(data),
            Self::Wayland(_) => None,
        }
    }

    pub fn x11_data_mut(&mut self) -> Option<&mut X11BackendData> {
        match self {
            Self::X11(data) => Some(data),
            Self::Wayland(_) => None,
        }
    }

    pub fn wayland_data(&self) -> Option<&WaylandBackendData> {
        match self {
            Self::X11(_) => None,
            Self::Wayland(data) => Some(data),
        }
    }

    pub fn wayland_data_mut(&mut self) -> Option<&mut WaylandBackendData> {
        match self {
            Self::X11(_) => None,
            Self::Wayland(data) => Some(data),
        }
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
            Backend::X11(data) => {
                BackendRef::X11(X11BackendRef::new(&data.conn, data.screen_num))
            }
            Backend::Wayland(data) => BackendRef::Wayland(&data.backend),
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

    fn pointer_location(&self) -> Option<(i32, i32)> {
        match self {
            BackendRef::X11(x11) => x11.pointer_location(),
            BackendRef::Wayland(wayland) => wayland.pointer_location(),
        }
    }

    fn warp_pointer(&self, x: f64, y: f64) {
        match self {
            BackendRef::X11(x11) => x11.warp_pointer(x, y),
            BackendRef::Wayland(wayland) => wayland.warp_pointer(x, y),
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

    fn set_monitor_config(&self, name: &str, config: &crate::config::config_toml::MonitorConfig) {
        match self {
            BackendRef::X11(x11) => x11.set_monitor_config(name, config),
            BackendRef::Wayland(wayland) => wayland.set_monitor_config(name, config),
        }
    }

    fn get_outputs(&self) -> Vec<BackendOutputInfo> {
        match self {
            BackendRef::X11(x11) => x11.get_outputs(),
            BackendRef::Wayland(wayland) => wayland.get_outputs(),
        }
    }
}
