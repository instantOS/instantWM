//! Backend abstraction.
//!
//! This module supports multiple window-system backends:
//! - **X11** — the original `x11rb`-based backend.
//! - **Wayland** — a Smithay-based Wayland compositor backend.

pub mod output;
pub mod wayland;
pub mod x11;

use crate::backend::x11::X11RuntimeConfig;
use crate::config::config_toml::VrrMode;
use crate::types::{Point, Rect, WindowId, XEmbedTray};
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

/// One region of the logical desktop as presented by the backend.
#[derive(Debug, Clone)]
pub struct BackendOutputInfo {
    pub name: String,
    pub rect: Rect,
    pub scale: f64,
    pub vrr_support: BackendVrrSupport,
    pub vrr_mode: Option<VrrMode>,
    pub vrr_enabled: bool,
    /// Other physical heads presenting this region. Backends report heads
    /// they drive as mirrors here instead of as separate outputs.
    pub mirrors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    X11,
    Wayland,
}

impl BackendKind {
    /// Apply a desktop wallpaper by spawning the platform's setter tool.
    ///
    /// Wayland compositors have no root pixmap, so sessions delegate to
    /// swaybg (restarting it if one is already running). X11 uses feh.
    /// Fire-and-forget: the child outlives the call either way.
    pub fn set_wallpaper(&self, path: &str) -> std::io::Result<()> {
        match self {
            Self::X11 => Command::new("feh")
                .arg("--bg-fill")
                .arg(path)
                .spawn()
                .map(|_| ()),
            Self::Wayland => {
                let _ = Command::new("killall").arg("swaybg").status();
                let spawned = Command::new("swaybg")
                    .arg("-i")
                    .arg(path)
                    .arg("-m")
                    .arg("fill")
                    .spawn();
                // Wayland has no SIGCHLD handler, so the replacement swaybg
                // must be handed to the dedicated reaper thread (see
                // [`BackendKind::reaps_children_via_signals`]) instead of
                // accumulating as a zombie on every wallpaper change.
                match spawned {
                    Ok(child) if !self.reaps_children_via_signals() => {
                        crate::util::reap_child_async(child);
                        Ok(())
                    }
                    result => result.map(|_| ()),
                }
            }
        }
    }

    /// External tool that lets the user drag out a screen rectangle, used by
    /// the `draw_window` action.
    ///
    /// Both tools are spawned with `-f x%xx%yx%wx%hx`, whose output
    /// [`crate::mouse::slop::parse_slop_output`] understands. X11 draws the
    /// selection through the instantOS helper on the root window; Wayland
    /// uses slurp's layer-shell overlay, which spans every output because
    /// the compositor implements wlr-layer-shell. Selection runs
    /// asynchronously — see [`crate::mouse::slop::spawn_region_selection`].
    pub fn region_selection_command(self) -> Option<Command> {
        match self {
            Self::X11 => Some(Command::new("instantslop")),
            Self::Wayland => Some(Command::new("slurp")),
        }
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

/// Window lifecycle and stacking effects shared by all backends.
pub trait WindowOps {
    fn resize_window(&mut self, window: WindowId, rect: Rect);
    /// Apply a backend-native border width when the backend has one.
    /// Compositor-rendered backends may implement this as a no-op.
    fn set_border_width(&mut self, window: WindowId, width: i32);
    fn raise_window_visual_only(&mut self, window: WindowId);
    fn apply_z_order(&mut self, windows: &[WindowId]);
    fn map_window(&mut self, window: WindowId);
    fn unmap_window(&mut self, window: WindowId);

    /// Check if a window still exists in the backend.
    ///
    /// Returns `true` if the window exists, `false` otherwise.
    /// This is a query method that returns state rather than performing an action.
    fn window_exists(&self, window: WindowId) -> bool;

    /// Return the protocol/backend surface type for a managed window.
    fn window_protocol(&self, window: WindowId) -> WindowProtocol;
    fn flush(&mut self);
}

/// Pointer queries and cursor movement.
pub trait PointerOps {
    /// Get current pointer location in root coordinates.
    ///
    /// Returns `None` if the pointer position cannot be determined
    /// (e.g., no pointer device available).
    fn pointer_location(&self) -> Option<Point>;

    /// Warp pointer to (x, y) in root coordinates.
    fn warp_pointer(&mut self, x: f64, y: f64);

    /// Warp to an integer logical point without repeating coordinate casts.
    fn warp_to_point(&mut self, point: Point) {
        self.warp_pointer(f64::from(point.x), f64::from(point.y));
    }
}

/// Reconcile native cursor and pointer routing with authoritative interaction
/// state.
///
/// Implementations must be idempotent. The shared interaction layer describes
/// the presentation it requires; native grabs and compositor cursor overrides
/// remain private backend mechanisms.
pub trait InteractionProjectionOps {
    fn reconcile_interaction_projection(
        &mut self,
        desired: crate::core_state::InteractionProjection,
    );
}

/// Graceful client termination projected through backend runtime state.
pub trait WindowCloseOps {
    fn close_window(&mut self, window: WindowId);
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

/// Output discovery.
pub trait OutputOps {
    /// Regions of the logical desktop, one per presenting output. Mirror heads
    /// the backend drives are listed in their source's
    /// [`BackendOutputInfo::mirrors`], not as outputs of their own.
    fn get_outputs(&self) -> Vec<BackendOutputInfo>;

    /// Physical heads known to the backend, including heads currently off.
    fn connected_output_names(&self) -> Vec<String>;
}

/// Native projection of the monitor policy.
///
/// Implemented on backend contexts rather than [`OutputOps`] because applying
/// policy updates backend-owned runtime state (automatic placement ownership,
/// the heads currently driven as mirrors).
pub trait OutputPolicyOps {
    /// Apply the complete, sanitized monitor policy. Backends resolve wildcard
    /// and named precedence atomically rather than exposing order-dependent
    /// setters.
    fn apply_monitor_configs(&mut self, policy: &crate::output_mirror::MonitorPolicy);
}

/// X11-specific backend data.
pub struct X11BackendData {
    pub conn: x11rb::rust_connection::RustConnection,
    pub screen_num: usize,
    pub x11_runtime: X11RuntimeConfig,
    pub xembed_tray: Option<XEmbedTray>,
}

/// Wayland-specific backend data.
#[derive(Default)]
pub struct WaylandBackendData {
    pub bar_renderer: crate::backend::wayland::bar::WaylandBarRenderer,
}

/// Backend identity is fixed by the runtime owner type.
pub trait BackendState {
    const KIND: BackendKind;
}
impl BackendState for X11BackendData {
    const KIND: BackendKind = BackendKind::X11;
}
impl BackendState for WaylandBackendData {
    const KIND: BackendKind = BackendKind::Wayland;
}
impl X11BackendData {
    pub fn new(conn: x11rb::rust_connection::RustConnection, screen_num: usize) -> Self {
        Self {
            conn,
            screen_num,
            x11_runtime: X11RuntimeConfig::default(),
            xembed_tray: None,
        }
    }
}
