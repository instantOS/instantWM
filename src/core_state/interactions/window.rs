use super::*;

/// What kind of drag interaction is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DragType {
    #[default]
    Move,
    /// Direct geometry resize for a floating client.
    Resize(ResizeDirection),
    /// Weight resize for a tiled leaf. The initial tree is stored alongside
    /// the private interaction state so motion is independent of event rate.
    TreeResize(ResizeDirection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResizePolicy {
    #[default]
    Free,
    PreserveAspect,
}

#[derive(Debug, Clone, Copy)]
pub struct ActiveResizeParams {
    pub win: WindowId,
    pub button: MouseButton,
    pub source: InteractionSource,
    pub direction: ResizeDirection,
    pub start: Point,
    pub geometry: Rect,
    pub policy: ResizePolicy,
}

#[derive(Debug, Clone)]
pub struct TreeResizeParams {
    pub win: WindowId,
    pub button: MouseButton,
    pub source: InteractionSource,
    pub direction: ResizeDirection,
    pub start: Point,
    pub geometry: Rect,
    pub origin: crate::layouts::tree::LayoutTree,
}

/// Operations to which an armed title-bar interaction may transition.
/// Tree resizing is deliberately absent because it must start with an
/// authoritative layout-tree snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmedDragType {
    Move,
    Resize(ResizeDirection),
}

/// Authoritative operation carried by an active drag.
///
/// Unlike [`DragType`], this owns all data required to execute the operation.
/// In particular, a tree resize cannot exist without the tree snapshot from
/// which pointer deltas are evaluated.
#[derive(Debug, Clone)]
pub(super) enum DragOperation {
    Move,
    Resize(ResizeDirection),
    TreeResize {
        direction: ResizeDirection,
        origin: crate::layouts::tree::LayoutTree,
    },
}

impl DragOperation {
    fn kind(&self) -> DragType {
        match self {
            Self::Move => DragType::Move,
            Self::Resize(direction) => DragType::Resize(*direction),
            Self::TreeResize { direction, .. } => DragType::TreeResize(*direction),
        }
    }
}

/// Borrowed view of an interaction operation for motion handlers.
#[derive(Debug, Clone, Copy)]
pub enum DragOperationRef<'a> {
    Move,
    Resize(ResizeDirection),
    TreeResize {
        direction: ResizeDirection,
        origin: &'a crate::layouts::tree::LayoutTree,
    },
}

#[derive(Debug, Clone)]
pub struct DragInteraction {
    win: WindowId,
    button: MouseButton,
    source: InteractionSource,
    origin: ArmedDragOrigin,
    operation: DragOperation,
    win_start_geo: Rect,
    start_point: Point,
    last_root_point: Point,
    /// Geometry to restore when the window is re-tiled (e.g. dropped on
    /// the bar).  For windows that were already floating this equals
    /// `win_start_geo`; for tiled windows promoted during the drag it
    /// preserves the saved float dimensions.
    drop_restore_geo: Rect,
    was_focused: bool,
    was_hidden: bool,
    suppress_click_action: bool,
    resize_policy: ResizePolicy,
}

impl DragInteraction {
    pub(super) fn immediate(
        win: WindowId,
        button: MouseButton,
        source: InteractionSource,
        operation: DragOperation,
        start: Point,
        geo: Rect,
    ) -> Self {
        Self {
            win,
            button,
            source,
            origin: ArmedDragOrigin::Client,
            operation,
            start_point: start,
            win_start_geo: geo,
            drop_restore_geo: geo,
            last_root_point: start,
            was_focused: false,
            was_hidden: false,
            suppress_click_action: false,
            resize_policy: ResizePolicy::Free,
        }
    }

    pub(super) fn armed(params: ArmedDragParams) -> Self {
        Self {
            win: params.win,
            button: params.button,
            source: params.source,
            origin: params.origin,
            operation: DragOperation::Move,
            start_point: params.start,
            win_start_geo: params.geometry,
            drop_restore_geo: params.restore_geometry,
            last_root_point: params.start,
            was_focused: params.was_focused,
            was_hidden: params.was_hidden,
            suppress_click_action: params.suppress_click_action,
            resize_policy: ResizePolicy::Free,
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
    pub fn drag_type(&self) -> DragType {
        self.operation.kind()
    }

    pub fn operation(&self) -> DragOperationRef<'_> {
        match &self.operation {
            DragOperation::Move => DragOperationRef::Move,
            DragOperation::Resize(direction) => DragOperationRef::Resize(*direction),
            DragOperation::TreeResize { direction, origin } => DragOperationRef::TreeResize {
                direction: *direction,
                origin,
            },
        }
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
    pub fn was_focused(&self) -> bool {
        self.was_focused
    }
    pub fn was_hidden(&self) -> bool {
        self.was_hidden
    }
    pub fn suppress_click_action(&self) -> bool {
        self.suppress_click_action
    }
    pub fn resize_policy(&self) -> ResizePolicy {
        self.resize_policy
    }

    pub(super) fn set_resize_policy(&mut self, policy: ResizePolicy) {
        self.resize_policy = policy;
    }

    pub(super) fn record_motion(&mut self, point: Point) {
        self.last_root_point = point;
    }

    pub(super) fn activate_as(&mut self, drag_type: ArmedDragType, start: Point, geo: Rect) {
        self.operation = match drag_type {
            ArmedDragType::Move => DragOperation::Move,
            ArmedDragType::Resize(direction) => DragOperation::Resize(direction),
        };
        self.start_point = start;
        self.last_root_point = start;
        self.win_start_geo = geo;
    }
}

#[derive(Debug, Clone)]
pub enum WindowDragState {
    Armed(DragInteraction),
    /// Bar-title drag reordering the title strip. Converts to
    /// [`Self::Active`] when the pointer leaves the title strip.
    Reordering(DragInteraction, TitleReorderDrag),
    Active(DragInteraction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("another interaction is already captured")]
pub struct InteractionAlreadyActive;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragNotArmed;

impl std::fmt::Display for DragNotArmed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no armed drag is available to activate")
    }
}

impl std::error::Error for DragNotArmed {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragCancelReason {
    WindowDestroyed,
    SessionLocked,
    InputDeviceRemoved,
    InputCaptureLost,
    TouchCancelled,
}

#[derive(Debug, Clone, Copy)]
pub struct ArmedDragParams {
    pub win: WindowId,
    pub button: MouseButton,
    pub source: InteractionSource,
    pub origin: ArmedDragOrigin,
    pub start: Point,
    pub geometry: Rect,
    pub restore_geometry: Rect,
    pub was_focused: bool,
    pub was_hidden: bool,
    pub suppress_click_action: bool,
}

/// Where an armed title-bar interaction was pressed.
///
/// Only bar-title presses may promote to a bar reorder drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmedDragOrigin {
    BarTitle,
    Client,
}

/// A bar-title drag that is reordering the title strip.
///
/// Order changes are committed live while the pointer stays on a title cell of
/// `monitor_id`; leaving the title strip converts the interaction into an
/// ordinary move drag. There is no rollback on release or cancel.
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

impl WindowDragState {
    pub fn interaction(&self) -> &DragInteraction {
        match self {
            Self::Armed(drag) | Self::Reordering(drag, _) | Self::Active(drag) => drag,
        }
    }
}
