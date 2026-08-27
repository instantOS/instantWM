use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResizePolicy {
    #[default]
    Free,
    PreserveAspect,
}

/// Everything required to begin a direct floating-window resize.
#[derive(Debug, Clone, Copy)]
pub struct DirectResizeStart {
    pub win: WindowId,
    pub button: MouseButton,
    pub source: InteractionSource,
    pub direction: ResizeDirection,
    pub start: Point,
    pub geometry: Rect,
    pub policy: ResizePolicy,
}

/// Everything required to begin a semantic tiled-tree resize.
#[derive(Debug, Clone)]
pub struct TreeResizeStart {
    pub win: WindowId,
    pub button: MouseButton,
    pub source: InteractionSource,
    pub direction: ResizeDirection,
    pub start: Point,
    pub geometry: Rect,
    pub origin: crate::layouts::tree::LayoutTree,
}

/// The operation executed by an active window drag.
///
/// Operation-specific data lives in its variant, so move interactions cannot
/// accidentally carry resize policy and tree resizes cannot exist without an
/// authoritative layout snapshot.
#[derive(Debug, Clone)]
pub(crate) enum ActiveWindowOperation {
    Move,
    DirectResize {
        direction: ResizeDirection,
        policy: ResizePolicy,
    },
    TreeResize {
        direction: ResizeDirection,
        origin: crate::layouts::tree::LayoutTree,
    },
}

impl ActiveWindowOperation {
    pub(crate) fn cursor(&self) -> AltCursor {
        match self {
            Self::Move => AltCursor::Move,
            Self::DirectResize { direction, .. } | Self::TreeResize { direction, .. } => {
                AltCursor::Resize(*direction)
            }
        }
    }

    pub(crate) fn is_direct_resize(&self) -> bool {
        matches!(self, Self::DirectResize { .. })
    }

    pub(crate) fn is_tree_resize(&self) -> bool {
        matches!(self, Self::TreeResize { .. })
    }
}

/// A title press that has not crossed the drag threshold.
///
/// No active operation is stored here because it is not known yet. The press
/// may remain a click, become a title reorder, move, or direct resize.
#[derive(Debug, Clone)]
pub struct ArmedWindowDrag {
    win: WindowId,
    button: MouseButton,
    source: InteractionSource,
    origin: ArmedDragOrigin,
    start_point: Point,
    last_root_point: Point,
    drop_restore_geo: Rect,
    was_focused: bool,
    was_hidden: bool,
    suppress_click_action: bool,
}

impl ArmedWindowDrag {
    pub(super) fn new(params: ArmedDragStart) -> Self {
        Self {
            win: params.win,
            button: params.button,
            source: params.source,
            origin: params.origin,
            start_point: params.start,
            last_root_point: params.start,
            drop_restore_geo: params.restore_geometry,
            was_focused: params.was_focused,
            was_hidden: params.was_hidden,
            suppress_click_action: params.suppress_click_action,
        }
    }

    pub(super) fn activate(
        self,
        operation: ActiveWindowOperation,
        start: Point,
        geometry: Rect,
    ) -> ActiveWindowDrag {
        ActiveWindowDrag {
            win: self.win,
            button: self.button,
            source: self.source,
            operation,
            win_start_geo: geometry,
            start_point: start,
            last_root_point: start,
            drop_restore_geo: self.drop_restore_geo,
        }
    }

    pub fn win(&self) -> WindowId {
        self.win
    }
    pub fn button(&self) -> MouseButton {
        self.button
    }
    pub fn source(&self) -> InteractionSource {
        self.source
    }
    pub fn origin(&self) -> ArmedDragOrigin {
        self.origin
    }
    pub fn start_point(&self) -> Point {
        self.start_point
    }
    pub fn last_root_point(&self) -> Point {
        self.last_root_point
    }
    pub fn was_focused(&self) -> bool {
        self.was_focused
    }
    pub fn was_hidden(&self) -> bool {
        self.was_hidden
    }
    pub fn suppress_click_action(&self) -> bool {
        self.suppress_click_action
    }
    pub(super) fn record_motion(&mut self, point: Point) {
        self.last_root_point = point;
    }
}

/// A window move or resize whose operation is fully determined.
#[derive(Debug, Clone)]
pub struct ActiveWindowDrag {
    win: WindowId,
    button: MouseButton,
    source: InteractionSource,
    operation: ActiveWindowOperation,
    win_start_geo: Rect,
    start_point: Point,
    last_root_point: Point,
    /// Geometry to restore when a moved window is re-tiled.
    drop_restore_geo: Rect,
}

impl ActiveWindowDrag {
    pub(super) fn immediate(
        win: WindowId,
        button: MouseButton,
        source: InteractionSource,
        operation: ActiveWindowOperation,
        start: Point,
        geometry: Rect,
    ) -> Self {
        Self {
            win,
            button,
            source,
            operation,
            win_start_geo: geometry,
            start_point: start,
            last_root_point: start,
            drop_restore_geo: geometry,
        }
    }

    pub fn win(&self) -> WindowId {
        self.win
    }
    pub fn button(&self) -> MouseButton {
        self.button
    }
    pub fn source(&self) -> InteractionSource {
        self.source
    }
    pub(crate) fn operation(&self) -> &ActiveWindowOperation {
        &self.operation
    }
    pub fn win_start_geo(&self) -> Rect {
        self.win_start_geo
    }
    pub fn start_point(&self) -> Point {
        self.start_point
    }
    pub fn last_root_point(&self) -> Point {
        self.last_root_point
    }
    pub fn drop_restore_geo(&self) -> Rect {
        self.drop_restore_geo
    }
    pub(super) fn record_motion(&mut self, point: Point) {
        self.last_root_point = point;
    }
}

#[derive(Debug, Clone)]
pub enum WindowDragState {
    Armed(ArmedWindowDrag),
    /// A bar-title drag that is still reordering the title strip. Leaving the
    /// strip consumes the armed state and creates an active move.
    Reordering(ArmedWindowDrag, TitleReorderDrag),
    Active(ActiveWindowDrag),
}

impl WindowDragState {
    pub fn button(&self) -> MouseButton {
        match self {
            Self::Armed(drag) | Self::Reordering(drag, _) => drag.button(),
            Self::Active(drag) => drag.button(),
        }
    }

    pub fn source(&self) -> InteractionSource {
        match self {
            Self::Armed(drag) | Self::Reordering(drag, _) => drag.source(),
            Self::Active(drag) => drag.source(),
        }
    }

    pub fn win(&self) -> WindowId {
        match self {
            Self::Armed(drag) | Self::Reordering(drag, _) => drag.win(),
            Self::Active(drag) => drag.win(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("another interaction is already captured")]
pub struct InteractionAlreadyActive;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("no armed drag is available to activate")]
pub struct DragNotArmed;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragCancelReason {
    WindowDestroyed,
    SessionLocked,
    InputDeviceRemoved,
    InputCaptureLost,
    TouchCancelled,
}

#[derive(Debug, Clone, Copy)]
pub struct ArmedDragStart {
    pub win: WindowId,
    pub button: MouseButton,
    pub source: InteractionSource,
    pub origin: ArmedDragOrigin,
    pub start: Point,
    pub restore_geometry: Rect,
    pub was_focused: bool,
    pub was_hidden: bool,
    pub suppress_click_action: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmedDragOrigin {
    BarTitle,
    Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleReorderDrag {
    monitor_id: MonitorId,
}

impl TitleReorderDrag {
    pub fn new(monitor_id: MonitorId) -> Self {
        Self { monitor_id }
    }
    pub fn monitor_id(&self) -> MonitorId {
        self.monitor_id
    }
}
