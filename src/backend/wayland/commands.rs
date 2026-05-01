//! Command queue for the Wayland backend.
//!
//! This module defines the [`WmCommand`] enum, which represents window
//! manager intents requested by the Wayland compositor. These commands
//! are processed asynchronously during the event loop tick, ensuring
//! unidirectional data flow and avoiding deadlocks.

use crate::types::WindowId;

/// Parameters for mapping a new window into the WM.
#[derive(Debug)]
pub struct MapWindowParams {
    pub win: WindowId,
    pub properties: crate::client::WindowProperties,
    pub initial_geo: Option<crate::types::Rect>,
    pub launch_pid: Option<u32>,
    pub launch_startup_id: Option<String>,
    pub x11_hints: Option<x11rb::properties::WmHints>,
    pub x11_size_hints: Option<x11rb::properties::WmSizeHints>,
    pub parent: Option<WindowId>,
}

/// Commands sent from the Wayland compositor to the core Window Manager.
#[derive(Debug)]
pub enum WmCommand {
    /// Request focus for a specific window.
    FocusWindow(WindowId),
    /// Raise a window in the Z-order.
    RaiseWindow(WindowId),
    /// Map a new window that was just created.
    MapWindow(MapWindowParams),
    /// Unmap/destroy a window.
    UnmapWindow(WindowId),
    /// Stop managing a window (e.g. it was closed).
    UnmanageWindow(WindowId),
    /// Request to activate a window (e.g. from xdg-activation).
    ActivateWindow(WindowId),
    /// Pointer motion event.
    PointerMotion { time_msec: u32 },
    /// Pointer button event.
    PointerButton {
        button: u32,
        state: smithay::backend::input::ButtonState,
        time_msec: u32,
    },
    /// Pointer axis event.
    PointerAxis {
        source: smithay::backend::input::AxisSource,
        horizontal: f64,
        vertical: f64,
        time_msec: u32,
    },
    /// Request an interactive move drag.
    BeginMove(WindowId),
    /// Request an interactive resize drag.
    BeginResize {
        win: WindowId,
        dir: crate::types::ResizeDirection,
    },
    /// Update a window's properties (title, class, etc.).
    UpdateProperties {
        win: WindowId,
        properties: crate::client::WindowProperties,
    },
    /// Update XWayland-specific policy (hints, transient_for, etc.).
    UpdateXWaylandPolicy {
        win: WindowId,
        hints: Option<x11rb::properties::WmHints>,
        size_hints: Option<x11rb::properties::WmSizeHints>,
        is_fullscreen: bool,
        is_hidden: bool,
        is_above: bool,
    },
    /// Update a window's actual committed size from the compositor.
    UpdateWindowSize { win: WindowId, w: i32, h: i32 },
    /// Request to change a window's maximized state.
    SetMaximized { win: WindowId, maximized: bool },
    /// Request to change a window's fullscreen state.
    SetFullscreen { win: WindowId, fullscreen: bool },
    /// Request to change a window's minimized/hidden state.
    SetMinimized { win: WindowId, minimized: bool },
    /// Request to show a scratchpad by name.
    ShowScratchpad(String),
    /// Update a window's floating geometry.
    SetWindowGeometry {
        win: WindowId,
        rect: crate::types::Rect,
    },
    /// Request a space sync (refresh layout and visibility).
    RequestSpaceSync,
    /// Request a bar redraw.
    RequestBarRedraw,
    /// Record a pending launch (to match future windows to pids).
    RecordPendingLaunch { pid: Option<u32> },
    /// Request to restore focus (e.g. after an overlay closed).
    RestoreFocus,
}
