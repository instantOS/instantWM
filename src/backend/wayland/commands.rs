//! Command queue for the Wayland backend.
//!
//! This module defines the [`WmCommand`] enum, which represents window
//! manager intents requested by the Wayland compositor. Most commands are
//! processed asynchronously during the event loop tick. Protocol state that a
//! configure immediately advertises must first be committed synchronously via
//! the transaction helpers in this module, so the model remains the sole
//! authority for every configure generated in the meantime.

use crate::types::WindowId;

/// Commit a fullscreen protocol request to authoritative WM state.
///
/// Returning a transition means the window is managed and the compositor may
/// acknowledge the requested state in an XDG/XWayland configure. The caller
/// must also project any synchronous geometry carried by that transition.
/// Layout is urgent because delaying it behind an existing animation would
/// leave protocol state and fullscreen geometry out of sync.
pub(crate) fn apply_fullscreen_request(
    core: &mut crate::core_state::CoreState,
    work: &mut crate::core_state::PendingWork,
    bar: &mut crate::bar::BarState,
    win: WindowId,
    fullscreen: bool,
) -> Option<crate::client::mode::FullscreenTransition> {
    let Some(transition) = core.model.set_fullscreen(win, fullscreen) else {
        return None;
    };

    if transition.changed() {
        work.layout.mark_monitor_urgent(transition.monitor_id());
        bar.mark_dirty();
    }
    Some(transition)
}

/// Commit a client maximize request to authoritative WM state.
pub(crate) fn apply_maximized_request(
    core: &mut crate::core_state::CoreState,
    work: &mut crate::core_state::PendingWork,
    bar: &mut crate::bar::BarState,
    win: WindowId,
    maximized: bool,
) -> Option<crate::client::mode::MaximizedTransition> {
    let transition = core.model.set_client_maximized(win, maximized)?;
    if transition.changed() {
        work.layout.mark_monitor_urgent(transition.monitor_id());
        bar.mark_dirty();
    }
    Some(transition)
}

/// Immediately project geometry that must accompany a fullscreen state
/// configure.
///
/// In particular, native clients must receive the restored floating size in
/// the same dispatch as the unfullscreen state. Deferring that resize until
/// layout would let a final fullscreen-sized commit look authoritative after
/// the model had already returned to floating mode.
pub(crate) fn apply_fullscreen_geometry(
    state: &mut crate::backend::wayland::compositor::WaylandState,
    win: WindowId,
    transition: crate::client::mode::FullscreenTransition,
) {
    match transition {
        crate::client::mode::FullscreenTransition::EnteredFromLayout { monitor_rect, .. }
        | crate::client::mode::FullscreenTransition::EnteredFromFloating { monitor_rect, .. }
        | crate::client::mode::FullscreenTransition::EnteredFromFakeFullscreen {
            monitor_rect,
            ..
        } => {
            state.configure_presentation_transition(win, monitor_rect);
        }
        crate::client::mode::FullscreenTransition::ExitedToFloating { restore_rect, .. } => {
            state.configure_presentation_transition(win, restore_rect);
        }
        crate::client::mode::FullscreenTransition::ExitedToMaximized { work_rect, .. } => {
            state.configure_presentation_transition(win, work_rect);
        }
        _ => {}
    }
}

/// Project geometry carried by a synchronous maximize transition.
pub(crate) fn apply_maximized_geometry(
    state: &mut crate::backend::wayland::compositor::WaylandState,
    win: WindowId,
    transition: crate::client::mode::MaximizedTransition,
) {
    match transition {
        crate::client::mode::MaximizedTransition::Entered { work_rect, .. } => {
            state.configure_presentation_transition(win, work_rect);
        }
        crate::client::mode::MaximizedTransition::ExitedToFloating { restore_rect, .. } => {
            state.configure_presentation_transition(win, restore_rect);
        }
        _ => {}
    }
}

/// Pointer motion data queued from backend input sources.
///
/// Backends must not update compositor pointer location before this command is
/// processed. Wayland pointer constraints can only be applied correctly when
/// the original motion and the current pointer location are evaluated together.
#[derive(Debug)]
pub enum PointerMotionCommand {
    Relative {
        dx: f64,
        dy: f64,
        dx_unaccel: f64,
        dy_unaccel: f64,
        time_msec: u32,
        time_usec: u64,
    },
    Absolute {
        x: f64,
        y: f64,
        time_msec: u32,
    },
    Warp {
        x: f64,
        y: f64,
        time_msec: u32,
    },
    Refresh {
        time_msec: u32,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct PointerButtonCommand {
    pub code: u32,
    pub state: smithay::backend::input::ButtonState,
    pub time_msec: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct PointerAxis {
    pub amount: Option<f64>,
    pub v120: Option<f64>,
    pub relative_direction: smithay::backend::input::AxisRelativeDirection,
}

#[derive(Debug, Clone, Copy)]
pub struct PointerAxisCommand {
    pub source: smithay::backend::input::AxisSource,
    pub horizontal: PointerAxis,
    pub vertical: PointerAxis,
    pub time_msec: u32,
}

/// Parameters for mapping a new window into the WM.
#[derive(Debug)]
pub struct MapWindowParams {
    pub win: WindowId,
    pub properties: crate::client::WindowProperties,
    pub initial_geo: Option<crate::types::Rect>,
    /// Whether the initial x/y came from an explicit X11 USPosition or
    /// PPosition hint. Native Wayland toplevel positions are always
    /// compositor-owned.
    pub initial_position_is_explicit: bool,
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
    PointerMotion(PointerMotionCommand),
    /// Pointer button event.
    PointerButton(PointerButtonCommand),
    /// Pointer axis event.
    PointerAxis(PointerAxisCommand),
    /// Request an interactive move drag.
    BeginMove(WindowId),
    /// Request an interactive resize drag.
    BeginResize {
        win: WindowId,
        dir: crate::types::ResizeDirection,
    },
    /// Cancel any compositor-driven move or resize interaction.
    CancelInteractiveDrag(crate::core_state::DragCancelReason),
    /// Update a window's properties (title, class, etc.).
    UpdateProperties {
        win: WindowId,
        properties: crate::client::WindowProperties,
    },
    /// Update the backend-neutral transient-parent relationship.
    UpdateTransientFor {
        win: WindowId,
        parent: Option<WindowId>,
    },
    /// Reconcile one complete XWayland policy snapshot.
    UpdateXWaylandPolicy {
        win: WindowId,
        update: crate::backend::x11::policy::XWaylandPolicyUpdate,
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
    /// Re-read layer-shell exclusive zones for every output and refresh each
    /// monitor's `available_rect`, `work_rect` and bar position. Triggers a
    /// layout pass and bar redraw if any monitor changed.
    SyncLayerExclusiveZones,
    /// Select a tag/workspace on a specific monitor by name and tag index (0-indexed).
    SelectTag {
        monitor_name: String,
        tag_index: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::{apply_fullscreen_request, apply_maximized_request};
    use crate::bar::BarState;
    use crate::core_state::{CoreState, PendingWork};
    use crate::types::{Client, ClientPlacement, Monitor, Rect, WindowId};

    #[test]
    fn fullscreen_request_updates_authoritative_state_before_acknowledgement() {
        let mut core = CoreState::default();
        let monitor_id = core.model.monitors.push(Monitor::default());
        let win = WindowId(40);
        core.model.insert_client(Client {
            win,
            monitor_id,
            ..Client::default()
        });
        let mut work = PendingWork::default();
        work.layout.clear();
        let mut bar = BarState::default();

        assert!(apply_fullscreen_request(&mut core, &mut work, &mut bar, win, true).is_some());

        assert!(core.model.client(win).unwrap().mode().is_true_fullscreen());
        assert!(work.layout.is_pending());
        assert!(work.layout.is_urgent());
        assert!(bar.needs_redraw());
    }

    #[test]
    fn fullscreen_request_rejects_unknown_window_without_side_effects() {
        let mut core = CoreState::default();
        let mut work = PendingWork::default();
        work.layout.clear();
        let mut bar = BarState::default();

        assert!(
            apply_fullscreen_request(&mut core, &mut work, &mut bar, WindowId(404), true).is_none()
        );
        assert!(!work.layout.is_pending());
        assert!(!bar.needs_redraw());
    }

    #[test]
    fn maximize_request_updates_model_and_schedules_projection_atomically() {
        let mut core = CoreState::default();
        let monitor_id = core.model.monitors.push(Monitor::default());
        let win = WindowId(42);
        core.model.insert_client(Client {
            win,
            monitor_id,
            ..Client::default()
        });
        let mut work = PendingWork::default();
        work.layout.clear();
        let mut bar = BarState::default();

        assert!(apply_maximized_request(&mut core, &mut work, &mut bar, win, true).is_some());

        let mode = core.model.client(win).unwrap().mode();
        assert!(mode.is_maximized());
        assert!(mode.is_protocol_maximized());
        assert!(work.layout.is_pending());
        assert!(work.layout.is_urgent());
        assert!(bar.needs_redraw());
    }

    #[test]
    fn unfullscreen_request_restores_floating_geometry_before_layout() {
        let mut core = CoreState::default();
        let monitor_id = core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1920, 1080),
            available_rect: Rect::new(0, 0, 1920, 1080),
            ..Monitor::default()
        });
        let win = WindowId(41);
        let floating_rect = Rect::new(200, 150, 960, 540);
        let fullscreen_rect = Rect::new(0, 0, 1920, 1080);
        let mut client = Client {
            win,
            monitor_id,
            geo: floating_rect,
            old_geo: floating_rect,
            border_width: 2,
            old_border_width: 2,
            ..Client::default()
        };
        client.set_placement(ClientPlacement::Floating);
        core.model.insert_client(client);
        let mut work = PendingWork::default();
        let mut bar = BarState::default();

        assert!(apply_fullscreen_request(&mut core, &mut work, &mut bar, win, true).is_some());
        crate::client::sync_client_geometry(&mut core.model, win, fullscreen_rect);
        assert!(apply_fullscreen_request(&mut core, &mut work, &mut bar, win, false).is_some());

        let client = core.model.client(win).unwrap();
        assert!(client.mode().is_normal_floating());
        assert_eq!(client.geo, floating_rect);
        assert_eq!(client.saved_floating_rect(), Some(floating_rect));
    }
}
