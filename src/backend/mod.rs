//! Backend abstraction.
//!
//! This module supports multiple window-system backends:
//! - **X11** — the original `x11rb`-based backend.
//! - **Wayland** — a Smithay-based Wayland compositor backend.

pub mod output;
pub mod wayland;
pub mod x11;

use crate::backend::wayland::WaylandBackend;
use crate::backend::x11::{X11BackendRef, X11RuntimeConfig};
use crate::config::config_toml::VrrMode;
use crate::types::{AltCursor, MouseButton, Point, Rect, WindowId, XEmbedTray};
use bincode::{Decode, Encode};
use std::process::Command;

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

impl BackendKind {
    /// Whether external X-based screen-selection tools (`instantslop`) can
    /// draw overlays usable by this backend.
    ///
    /// `instantslop` selects a region by drawing on the X root window, which
    /// spans every output only under the native X11 backend. Under Wayland
    /// the compositor owns the root; an equivalent would require a
    /// layer-shell overlay and has not been built.
    pub fn supports_x_selection_tools(self) -> bool {
        matches!(self, Self::X11)
    }

    /// Whether this backend reaps child processes via a SIGCHLD handler on
    /// its main-loop thread (`backend/x11/startup.rs`). Backends without one
    /// must hand spawned children to the dedicated reaper thread instead,
    /// or short-lived scripts accumulate as zombies.
    pub fn reaps_children_via_signals(self) -> bool {
        matches!(self, Self::X11)
    }
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
        root: Point,
        /// Modifier key mask (X11: `state` field, Wayland: modifier flags).
        modifiers: u32,
    },
    /// Button press (start of a click).
    ButtonPress { button: MouseButton },
    /// Button release.
    ButtonRelease {
        button: MouseButton,
        /// Modifier mask at release time.
        modifiers: u32,
    },
    /// Key press (used with `with_keys: true`).
    KeyPress { keycode: u32 },
}

/// Window lifecycle and stacking effects shared by all backends.
pub trait WindowOps {
    fn resize_window(&self, window: WindowId, rect: Rect);
    /// Apply a backend-native border width when the backend has one.
    /// Compositor-rendered backends may implement this as a no-op.
    fn set_border_width(&self, window: WindowId, width: i32);
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

    /// Return the protocol/backend surface type for a managed window.
    fn window_protocol(&self, window: WindowId) -> WindowProtocol;
    fn flush(&self);
}

/// Pointer queries and cursor movement.
pub trait PointerOps {
    /// Get current pointer location in root coordinates.
    ///
    /// Returns `None` if the pointer position cannot be determined
    /// (e.g., no pointer device available).
    fn pointer_location(&self) -> Option<Point>;

    /// Warp pointer to (x, y) in root coordinates.
    fn warp_pointer(&self, x: f64, y: f64);

    /// Warp to an integer logical point without repeating coordinate casts.
    fn warp_to_point(&self, point: Point) {
        self.warp_pointer(f64::from(point.x), f64::from(point.y));
    }
}

/// Backend projection of the WM-owned cursor presentation.
///
/// Implemented by backend contexts rather than bare connections because X11
/// cursor resources live in its runtime state.
pub trait CursorOps {
    fn apply_cursor_style(&mut self, style: AltCursor);
}

/// Graceful client termination projected through backend runtime state.
pub trait WindowCloseOps {
    fn close_window(&mut self, window: WindowId);
}

/// Backend effects associated with the lifetime of a user-driven resize.
///
/// X11 implements this as a no-op because its synchronous pointer grab owns
/// the resize lifetime. Wayland projects the lifetime into xdg-toplevel's
/// `resizing` state.
pub trait InteractiveResizeOps {
    fn begin_interactive_resize(&self, window: WindowId);
    fn end_interactive_resize(&self, window: WindowId);
}

/// Backend effects used by compositor-owned modal interactions.
///
/// Core state remains authoritative in `WmCtx`; implementations only acquire
/// or release backend input ownership and project preview state for rendering.
pub trait LayoutInteractionOps {
    fn begin_modal_keyboard(&mut self) -> bool;
    fn end_modal_keyboard(&mut self);
    fn layout_preview_changed(
        &mut self,
        rect: Option<Rect>,
        style: crate::types::InteractionOutlineStyle,
        target: Option<crate::types::WindowId>,
        animate: bool,
        duration: std::time::Duration,
    );
}

/// Output discovery and configuration.
pub trait OutputOps {
    /// Set monitor configuration. Every active backend owns output policy.
    fn set_monitor_config(&self, name: &str, config: &crate::config::config_toml::MonitorConfig);

    /// Get current outputs from the backend.
    fn get_outputs(&self) -> Vec<BackendOutputInfo>;

    /// Legacy fallback discovery when primary discovery reports a single
    /// placeholder screen. X11 consults Xinerama; backends without a
    /// secondary discovery protocol return `None`.
    fn query_fallback_outputs(&self) -> Option<Vec<BackendOutputInfo>> {
        let _ = self;
        None
    }
}

/// X11-specific backend data.
pub struct X11BackendData {
    pub conn: x11rb::rust_connection::RustConnection,
    pub screen_num: usize,
    pub x11_runtime: X11RuntimeConfig,
    pub xembed_tray: Option<XEmbedTray>,
}

/// Wayland-specific backend data.
pub struct WaylandBackendData {
    pub backend: WaylandBackend,
    pub bar_painter: crate::backend::wayland::bar::WaylandBarPainter,
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
            xembed_tray: None,
        }))
    }

    pub fn new_wayland(backend: WaylandBackend) -> Self {
        Self::Wayland(Box::new(WaylandBackendData {
            backend,
            bar_painter: crate::backend::wayland::bar::WaylandBarPainter::default(),
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

    /// Apply a desktop wallpaper by spawning the platform's setter tool.
    ///
    /// Wayland compositors have no root pixmap, so sessions delegate to
    /// swaybg (restarting it if one is already running). X11 uses feh.
    /// Fire-and-forget: the child outlives the call either way.
    pub fn set_wallpaper(&self, path: &str) -> std::io::Result<()> {
        match self {
            Self::X11(_) => Command::new("feh")
                .arg("--bg-fill")
                .arg(path)
                .spawn()
                .map(|_| ()),
            Self::Wayland(_) => {
                let _ = Command::new("killall").arg("swaybg").status();
                Command::new("swaybg")
                    .arg("-i")
                    .arg(path)
                    .arg("-m")
                    .arg("fill")
                    .spawn()
                    .map(|_| ())
            }
        }
    }

    pub fn kind(&self) -> BackendKind {
        match self {
            Self::X11(_) => BackendKind::X11,
            Self::Wayland(_) => BackendKind::Wayland,
        }
    }
}

impl WindowOps for Backend {
    fn resize_window(&self, window: WindowId, rect: Rect) {
        match self {
            Backend::X11(data) => {
                X11BackendRef::new(&data.conn, data.screen_num).resize_window(window, rect)
            }
            Backend::Wayland(data) => data.backend.resize_window(window, rect),
        }
    }

    fn set_border_width(&self, window: WindowId, width: i32) {
        match self {
            Backend::X11(data) => {
                X11BackendRef::new(&data.conn, data.screen_num).set_border_width(window, width)
            }
            Backend::Wayland(data) => data.backend.set_border_width(window, width),
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

    fn window_protocol(&self, window: WindowId) -> WindowProtocol {
        match self {
            Backend::X11(data) => {
                X11BackendRef::new(&data.conn, data.screen_num).window_protocol(window)
            }
            Backend::Wayland(data) => data.backend.window_protocol(window),
        }
    }

    fn flush(&self) {
        match self {
            Backend::X11(data) => X11BackendRef::new(&data.conn, data.screen_num).flush(),
            Backend::Wayland(data) => data.backend.flush(),
        }
    }
}

impl PointerOps for Backend {
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
}

impl OutputOps for Backend {
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

    fn query_fallback_outputs(&self) -> Option<Vec<BackendOutputInfo>> {
        match self {
            Backend::X11(data) => {
                X11BackendRef::new(&data.conn, data.screen_num).query_fallback_outputs()
            }
            Backend::Wayland(data) => data.backend.query_fallback_outputs(),
        }
    }
}
