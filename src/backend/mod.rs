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
use crate::config::config_toml::VrrMode;
use crate::types::{
    MouseButton, Point, Rect, Systray, WaylandSystray, WaylandSystrayMenu, WindowId,
};
use bincode::{Decode, Encode};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Encode, Decode,
)]
pub enum BackendVrrSupport {
    Unsupported,
    RequiresModeset,
    Supported,
}

#[derive(Debug, Clone)]
pub struct BackendOutputInfo {
    pub name: String,
    pub rect: Rect,
    pub scale: f64,
    pub vrr_support: BackendVrrSupport,
    pub vrr_mode: Option<VrrMode>,
    pub vrr_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    X11,
    Wayland,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Encode, Decode,
)]
#[serde(rename_all = "snake_case")]
pub enum WindowProtocol {
    Unknown,
    X11,
    Wayland,
    #[serde(rename = "xwayland")]
    XWayland,
}

/// Backend-agnostic event type for drag loops.
///
/// Backend-specific events (X11 `x11rb::protocol::Event`, Wayland input
/// events) are converted to this enum so that shared code does not depend
/// on either backend's event types.
#[derive(Debug, Clone, PartialEq)]
pub enum BackendEvent {
    /// Pointer motion.
    Motion {
        root_x: f64,
        root_y: f64,
        /// Modifier key mask (X11: `state` field, Wayland: modifier flags).
        modifiers: u32,
    },
    /// Button press (start of a click).
    ButtonPress { button: MouseButton },
    /// Button release.
    ButtonRelease { button: MouseButton },
    /// Key press (used with `with_keys: true`).
    KeyPress { keycode: u32 },
}

/// Core backend operations required by the WM.
pub trait BackendOps {
    fn resize_window(&self, window: WindowId, rect: Rect);
    fn raise_window_visual_only(&self, window: WindowId);
    fn apply_z_order(&self, windows: &[WindowId]);
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
    fn pointer_location(&self) -> Option<Point>;

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

    /// Return the protocol/backend surface type for a managed window.
    fn window_protocol(&self, _window: WindowId) -> WindowProtocol {
        WindowProtocol::Unknown
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

    /// Get list of input devices (Wayland only)
    fn get_input_devices(&self) -> Vec<String> {
        Vec::new()
    }

    /// Position and resize a window directly (no size-hint enforcement).
    ///
    /// X11-only operation. The Wayland backend leaves this as a no-op because
    /// compositor-side geometry is authoritative there.
    fn configure_window_geometry(&self, _win: WindowId, _rect: Rect) {}
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
    pub wayland_systray_runtime: Option<crate::backend::wayland::systray::WaylandSystrayRuntime>,
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

    pub fn get_input_devices(&self) -> Vec<String> {
        match self {
            Self::X11(_) => Vec::new(),
            Self::Wayland(data) => data.backend.get_input_devices(),
        }
    }

    pub fn get_outputs(&self) -> Vec<BackendOutputInfo> {
        match self {
            Self::X11(data) => X11BackendRef::new(&data.conn, data.screen_num).get_outputs(),
            Self::Wayland(data) => data.backend.get_outputs(),
        }
    }

    pub fn kind(&self) -> BackendKind {
        match self {
            Self::X11(_) => BackendKind::X11,
            Self::Wayland(_) => BackendKind::Wayland,
        }
    }
}

impl BackendOps for Backend {
    fn resize_window(&self, window: WindowId, rect: Rect) {
        match self {
            Backend::X11(data) => {
                X11BackendRef::new(&data.conn, data.screen_num).resize_window(window, rect)
            }
            Backend::Wayland(data) => data.backend.resize_window(window, rect),
        }
    }

    fn configure_window_geometry(&self, window: WindowId, rect: Rect) {
        match self {
            Backend::X11(data) => X11BackendRef::new(&data.conn, data.screen_num)
                .configure_window_geometry(window, rect),
            Backend::Wayland(data) => data.backend.configure_window_geometry(window, rect),
        }
    }

    fn raise_window_visual_only(&self, window: WindowId) {
        match self {
            Backend::X11(data) => {
                X11BackendRef::new(&data.conn, data.screen_num).raise_window_visual_only(window)
            }
            Backend::Wayland(data) => data.backend.raise_window_visual_only(window),
        }
    }

    fn apply_z_order(&self, windows: &[WindowId]) {
        match self {
            Backend::X11(data) => {
                X11BackendRef::new(&data.conn, data.screen_num).apply_z_order(windows)
            }
            Backend::Wayland(data) => data.backend.apply_z_order(windows),
        }
    }

    fn set_focus(&self, window: WindowId) {
        match self {
            Backend::X11(data) => X11BackendRef::new(&data.conn, data.screen_num).set_focus(window),
            Backend::Wayland(data) => data.backend.set_focus(window),
        }
    }

    fn map_window(&self, window: WindowId) {
        match self {
            Backend::X11(data) => {
                X11BackendRef::new(&data.conn, data.screen_num).map_window(window)
            }
            Backend::Wayland(data) => data.backend.map_window(window),
        }
    }

    fn unmap_window(&self, window: WindowId) {
        match self {
            Backend::X11(data) => {
                X11BackendRef::new(&data.conn, data.screen_num).unmap_window(window)
            }
            Backend::Wayland(data) => data.backend.unmap_window(window),
        }
    }

    fn window_exists(&self, window: WindowId) -> bool {
        match self {
            Backend::X11(data) => {
                X11BackendRef::new(&data.conn, data.screen_num).window_exists(window)
            }
            Backend::Wayland(data) => data.backend.window_exists(window),
        }
    }

    fn flush(&self) {
        match self {
            Backend::X11(data) => X11BackendRef::new(&data.conn, data.screen_num).flush(),
            Backend::Wayland(data) => data.backend.flush(),
        }
    }

    fn pointer_location(&self) -> Option<Point> {
        match self {
            Backend::X11(data) => {
                X11BackendRef::new(&data.conn, data.screen_num).pointer_location()
            }
            Backend::Wayland(data) => data.backend.pointer_location(),
        }
    }

    fn warp_pointer(&self, x: f64, y: f64) {
        match self {
            Backend::X11(data) => {
                X11BackendRef::new(&data.conn, data.screen_num).warp_pointer(x, y)
            }
            Backend::Wayland(data) => data.backend.warp_pointer(x, y),
        }
    }

    fn window_title(&self, window: WindowId) -> Option<String> {
        match self {
            Backend::X11(data) => {
                X11BackendRef::new(&data.conn, data.screen_num).window_title(window)
            }
            Backend::Wayland(data) => data.backend.window_title(window),
        }
    }

    fn window_protocol(&self, window: WindowId) -> WindowProtocol {
        match self {
            Backend::X11(data) => {
                X11BackendRef::new(&data.conn, data.screen_num).window_protocol(window)
            }
            Backend::Wayland(data) => data.backend.window_protocol(window),
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
            Backend::X11(data) => X11BackendRef::new(&data.conn, data.screen_num)
                .set_keyboard_layout(layout, variant, options, model),
            Backend::Wayland(data) => data
                .backend
                .set_keyboard_layout(layout, variant, options, model),
        }
    }

    fn set_monitor_config(&self, name: &str, config: &crate::config::config_toml::MonitorConfig) {
        match self {
            Backend::X11(data) => {
                X11BackendRef::new(&data.conn, data.screen_num).set_monitor_config(name, config)
            }
            Backend::Wayland(data) => data.backend.set_monitor_config(name, config),
        }
    }

    fn get_outputs(&self) -> Vec<BackendOutputInfo> {
        match self {
            Backend::X11(data) => X11BackendRef::new(&data.conn, data.screen_num).get_outputs(),
            Backend::Wayland(data) => data.backend.get_outputs(),
        }
    }
}
