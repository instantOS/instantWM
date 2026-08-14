use crate::actions::ButtonAction;

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
enum DragOperation {
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
    fn immediate(
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

    fn armed(params: ArmedDragParams) -> Self {
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

    fn set_resize_policy(&mut self, policy: ResizePolicy) {
        self.resize_policy = policy;
    }

    fn record_motion(&mut self, point: Point) {
        self.last_root_point = point;
    }

    fn activate_as(&mut self, drag_type: ArmedDragType, start: Point, geo: Rect) {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragAlreadyActive;

impl std::fmt::Display for DragAlreadyActive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("another interactive drag is already armed or active")
    }
}

impl std::error::Error for DragAlreadyActive {}

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

/// On X11, the synchronous grab loop drives this. On Wayland, the calloop
/// press/motion/release events drive it asynchronously.
#[derive(Debug, Clone)]
pub struct TagDragState {
    /// The initial tag mask that was clicked.
    pub initial_tag: TagMask,
    /// Pointer position at press time, used to distinguish a click from a drag.
    pub start: Point,
    /// Whether pointer motion has crossed the drag threshold.
    pub dragging: bool,
    /// Monitor ID where the drag started.
    pub monitor_id: MonitorId,
    /// Last seen tag gesture index (None = none).
    pub last_tag: Option<usize>,
    /// Whether cursor is still on the bar.
    pub cursor_on_bar: bool,
    /// Last motion coordinates + modifier state (for release handling).
    pub last_motion: Option<(Point, u32)>,
    /// The mouse button that started the drag.
    pub button: MouseButton,
    /// Input stream that owns this interaction.
    pub source: InteractionSource,
}

/// Direction latched by a bottom-bar swipe.
///
/// Horizontal swipes keep their previous semantics (left = previous tag,
/// right = next tag); a mostly-upward swipe latches [`Self::Up`] instead, so a
/// press-hold-slide-release leaving the bar can trigger a third binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwipeDirection {
    Left,
    Right,
    Up,
}

/// All configurable actions for a bottom-bar press-hold-slide-release gesture.
#[derive(Debug, Clone)]
pub struct BottomBarActions {
    pub left: Box<ButtonAction>,
    pub right: Box<ButtonAction>,
    pub up: Box<ButtonAction>,
    pub click: Box<ButtonAction>,
    pub hold: Box<ButtonAction>,
}

/// Swipe or tap on the bottom-bar gesture strip.
///
/// Once the pointer travels at least `threshold` pixels from the press
/// position, the swipe direction is latched (rightward = `right` action,
/// leftward = `left` action, mostly upward = `up` action) and locked in until
/// release. If no direction latches, release distinguishes a quick **click**
/// (short press) from a **hold** (long press) by duration and fires that action
/// instead. Exactly one action fires per press-hold-slide-release, regardless
/// of how far or long the drag goes.
#[derive(Debug, Clone)]
pub struct BottomBarDrag {
    button: MouseButton,
    source: InteractionSource,
    monitor_id: MonitorId,
    anchor_x: i32,
    anchor_y: i32,
    threshold: i32,
    press_time_msec: u32,
    actions: BottomBarActions,
    /// Swipe direction latched once the pointer travels at least `threshold`
    /// pixels from the press position.
    direction: Option<SwipeDirection>,
}

impl BottomBarDrag {
    pub fn new(
        button: MouseButton,
        source: InteractionSource,
        monitor_id: MonitorId,
        anchor: Point,
        threshold: i32,
        press_time_msec: u32,
        actions: BottomBarActions,
    ) -> Self {
        Self {
            button,
            source,
            monitor_id,
            anchor_x: anchor.x,
            anchor_y: anchor.y,
            threshold: threshold.max(1),
            press_time_msec,
            actions,
            direction: None,
        }
    }

    pub fn button(&self) -> MouseButton {
        self.button
    }

    pub fn source(&self) -> InteractionSource {
        self.source
    }

    pub fn monitor_id(&self) -> MonitorId {
        self.monitor_id
    }

    pub fn press_time_msec(&self) -> u32 {
        self.press_time_msec
    }

    pub fn left(&self) -> &ButtonAction {
        &self.actions.left
    }

    pub fn right(&self) -> &ButtonAction {
        &self.actions.right
    }

    pub fn up(&self) -> &ButtonAction {
        &self.actions.up
    }

    pub fn click(&self) -> &ButtonAction {
        &self.actions.click
    }

    pub fn hold(&self) -> &ButtonAction {
        &self.actions.hold
    }

    /// The swipe direction latched so far, if any.
    pub fn latched_direction(&self) -> Option<SwipeDirection> {
        self.direction
    }

    /// Observe pointer motion, latching the swipe direction the first time the
    /// pointer travels at least `threshold` pixels from the press position.
    ///
    /// The dominant axis wins: a mostly-upward drag latches [`SwipeDirection::Up`]
    /// (up = `anchor_y - root.y`), anything else latches left/right from the
    /// horizontal displacement. Returns the newly latched direction, or `None`
    /// when the motion is still below the threshold or the direction was
    /// already latched. The latch is never updated afterwards, so a single
    /// swipe always maps to one action.
    pub fn update(&mut self, root: Point) -> Option<SwipeDirection> {
        if self.direction.is_some() {
            return None;
        }
        let dx = root.x - self.anchor_x;
        let up = self.anchor_y - root.y;
        if dx.abs() < self.threshold && up < self.threshold {
            return None;
        }
        let direction = if up >= dx.abs() {
            SwipeDirection::Up
        } else if dx > 0 {
            SwipeDirection::Right
        } else {
            SwipeDirection::Left
        };
        self.direction = Some(direction);
        self.direction
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarVolumeDrag {
    button: MouseButton,
    source: InteractionSource,
    monitor_id: MonitorId,
    anchor_y: i32,
    threshold: i32,
}

/// A compositor-owned press on an overview card.
///
/// The complete input sequence is captured so neither the press nor release
/// can leak to the client selected when overview closes. Gesture semantics are
/// resolved from the final displacement on release: a tap selects the card,
/// a predominantly upward drag closes it, and other drags are cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverviewCardDrag {
    window: WindowId,
    button: MouseButton,
    source: InteractionSource,
    start: Point,
    last: Point,
    threshold: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverviewCardAction {
    Select(WindowId),
    Close(WindowId),
    Cancel,
}

impl OverviewCardDrag {
    pub fn new(
        window: WindowId,
        button: MouseButton,
        source: InteractionSource,
        start: Point,
        threshold: i32,
    ) -> Self {
        Self {
            window,
            button,
            source,
            start,
            last: start,
            threshold: threshold.max(1),
        }
    }

    pub fn window(self) -> WindowId {
        self.window
    }

    pub fn button(self) -> MouseButton {
        self.button
    }

    pub fn source(self) -> InteractionSource {
        self.source
    }

    /// Record motion and report a close-threshold transition, if one occurred.
    pub fn update(&mut self, root: Point) -> Option<bool> {
        let was_armed = self.close_armed();
        self.last = root;
        let is_armed = self.close_armed();
        (was_armed != is_armed).then_some(is_armed)
    }

    pub fn close_armed(self) -> bool {
        let dx = self.last.x - self.start.x;
        let up = self.start.y - self.last.y;
        up >= self.threshold && up >= dx.abs()
    }

    pub fn action(self) -> OverviewCardAction {
        let dx = self.last.x - self.start.x;
        let dy = self.last.y - self.start.y;
        if dx.abs() < self.threshold && dy.abs() < self.threshold {
            OverviewCardAction::Select(self.window)
        } else if self.close_armed() {
            OverviewCardAction::Close(self.window)
        } else {
            OverviewCardAction::Cancel
        }
    }
}

impl SidebarVolumeDrag {
    pub fn new(
        button: MouseButton,
        source: InteractionSource,
        monitor_id: MonitorId,
        anchor_y: i32,
        threshold: i32,
    ) -> Self {
        Self {
            button,
            source,
            monitor_id,
            anchor_y,
            threshold: threshold.max(1),
        }
    }

    pub fn button(self) -> MouseButton {
        self.button
    }

    pub fn source(self) -> InteractionSource {
        self.source
    }

    pub fn monitor_id(self) -> MonitorId {
        self.monitor_id
    }

    /// Consume pointer distance and return signed volume steps.
    ///
    /// Positive values mean volume-up. Advancing the anchor only by complete
    /// thresholds preserves sub-threshold movement across input events and
    /// makes the result independent of backend motion-event compression.
    pub fn update(&mut self, root_y: i32) -> i32 {
        let delta = self.anchor_y - root_y;
        let steps = delta / self.threshold;
        self.anchor_y -= steps * self.threshold;
        steps
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HotCornerState {
    #[default]
    Ready,
    Latched {
        monitor_id: MonitorId,
    },
}

impl HotCornerState {
    /// Update the hot-corner latch. Returns the monitor whose corner was
    /// entered when the caller should toggle the edge scratchpad.
    pub fn update(
        &mut self,
        monitor_id: Option<MonitorId>,
        inside_activation_zone: bool,
        inside_keep_zone: bool,
    ) -> Option<MonitorId> {
        match *self {
            Self::Ready => {
                let monitor_id = monitor_id.filter(|_| inside_activation_zone)?;
                *self = Self::Latched { monitor_id };
                Some(monitor_id)
            }
            Self::Latched {
                monitor_id: latched_monitor,
            } if monitor_id == Some(latched_monitor) && inside_keep_zone => None,
            Self::Latched { .. } => {
                // Rearm only. Requiring a subsequent sample before activation
                // prevents a single jump between monitor corners from firing
                // two actions at once.
                *self = Self::Ready;
                None
            }
        }
    }
}

#[cfg(test)]
mod pointer_interaction_tests {
    use super::{
        BottomBarDrag, DragState, HotCornerState, OverviewCardAction, OverviewCardDrag,
        SidebarVolumeDrag, SwipeDirection,
    };
    use crate::actions::ButtonAction;
    use crate::types::{InteractionSource, MonitorId, MouseButton, Point, Rect, WindowId};

    fn bottom_bar_drag(anchor_x: i32, anchor_y: i32) -> BottomBarDrag {
        BottomBarDrag::new(
            MouseButton::Left,
            InteractionSource::Pointer,
            MonitorId::from_raw(3),
            Point::new(anchor_x, anchor_y),
            30,
            0,
            super::BottomBarActions {
                left: Box::new(ButtonAction::named(crate::actions::NamedAction::ScrollLeft)),
                right: Box::new(ButtonAction::named(
                    crate::actions::NamedAction::ScrollRight,
                )),
                up: Box::new(ButtonAction::named(
                    crate::actions::NamedAction::ToggleOverview,
                )),
                click: Box::new(ButtonAction::named(crate::actions::NamedAction::Spawn)),
                hold: Box::new(ButtonAction::named(
                    crate::actions::NamedAction::ToggleOverview,
                )),
            },
        )
    }

    #[test]
    fn bottom_bar_swipe_latches_direction_at_threshold() {
        let mut drag = bottom_bar_drag(500, 1000);

        assert_eq!(drag.update(Point::new(529, 1000)), None); // 29px < 30: not yet
        assert_eq!(
            drag.update(Point::new(530, 1000)),
            Some(SwipeDirection::Right)
        );
        assert_eq!(drag.update(Point::new(700, 1000)), None); // already latched
        assert_eq!(drag.latched_direction(), Some(SwipeDirection::Right));

        let mut left = bottom_bar_drag(500, 1000);
        assert_eq!(
            left.update(Point::new(470, 1000)),
            Some(SwipeDirection::Left)
        );
        assert_eq!(left.latched_direction(), Some(SwipeDirection::Left));

        let mut up = bottom_bar_drag(500, 1000);
        assert_eq!(up.update(Point::new(500, 970)), Some(SwipeDirection::Up));
        assert_eq!(up.latched_direction(), Some(SwipeDirection::Up));
    }

    #[test]
    fn bottom_bar_swipe_dominant_axis_wins() {
        // A mostly-upward drag latches Up even though horizontal travel exists.
        let mut drag = bottom_bar_drag(500, 1000);
        assert_eq!(drag.update(Point::new(520, 960)), Some(SwipeDirection::Up));
        assert_eq!(drag.latched_direction(), Some(SwipeDirection::Up));

        // A mostly-rightward drag with some upward travel latches Right.
        let mut drag = bottom_bar_drag(500, 1000);
        assert_eq!(
            drag.update(Point::new(560, 990)),
            Some(SwipeDirection::Right)
        );
    }

    #[test]
    fn bottom_bar_swipe_direction_is_locked_after_first_crossing() {
        let mut drag = bottom_bar_drag(500, 1000);

        assert_eq!(
            drag.update(Point::new(560, 1000)),
            Some(SwipeDirection::Right)
        );
        // Reversing past the threshold on the other side must not re-latch.
        assert_eq!(drag.update(Point::new(300, 100)), None);
        assert_eq!(drag.latched_direction(), Some(SwipeDirection::Right));
    }

    #[test]
    fn bottom_bar_drag_exposes_bound_directional_actions() {
        let drag = bottom_bar_drag(100, 1000);
        assert!(matches!(
            drag.left(),
            ButtonAction::Named {
                action: crate::actions::NamedAction::ScrollLeft,
                ..
            }
        ));
        assert!(matches!(
            drag.right(),
            ButtonAction::Named {
                action: crate::actions::NamedAction::ScrollRight,
                ..
            }
        ));
        assert!(matches!(
            drag.up(),
            ButtonAction::Named {
                action: crate::actions::NamedAction::ToggleOverview,
                ..
            }
        ));
    }

    #[test]
    fn bottom_bar_lifecycle_rejects_overlap_and_wrong_button_release() {
        let mut interactions = DragState::default();
        interactions
            .begin_overview_card(OverviewCardDrag::new(
                WindowId(1),
                MouseButton::Left,
                InteractionSource::Pointer,
                Point::new(0, 0),
                10,
            ))
            .unwrap();
        assert!(
            interactions
                .begin_bottom_bar(bottom_bar_drag(500, 1000))
                .is_err()
        );
        assert!(!interactions.bottom_bar_gesture_active());

        interactions.cancel_capture();
        interactions
            .begin_bottom_bar(bottom_bar_drag(500, 1000))
            .unwrap();
        assert!(interactions.captured_source() == Some(InteractionSource::Pointer));
        assert!(!interactions.finish_bottom_bar(MouseButton::Right));
        assert!(interactions.bottom_bar_gesture_active());
        assert!(interactions.finish_bottom_bar(MouseButton::Left));
        assert!(!interactions.bottom_bar_gesture_active());
    }

    #[test]
    fn bottom_bar_cancel_clears_the_gesture() {
        let mut interactions = DragState::default();
        interactions
            .begin_bottom_bar(bottom_bar_drag(500, 1000))
            .unwrap();
        assert!(interactions.cancel_bottom_bar());
        assert!(!interactions.bottom_bar_gesture_active());
        assert!(!interactions.cancel_bottom_bar());
    }

    #[test]
    fn hot_corner_fires_once_until_pointer_leaves_keep_zone() {
        let monitor = MonitorId::from_raw(7);
        let mut state = HotCornerState::default();

        assert_eq!(state.update(Some(monitor), true, true), Some(monitor));
        assert_eq!(state.update(Some(monitor), true, true), None);
        assert_eq!(state.update(Some(monitor), false, true), None);
        assert_eq!(state.update(Some(monitor), false, false), None);
        assert_eq!(state.update(Some(monitor), true, true), Some(monitor));
    }

    #[test]
    fn hot_corner_rearms_without_firing_when_pointer_jumps_between_monitors() {
        let first = MonitorId::from_raw(1);
        let second = MonitorId::from_raw(2);
        let mut state = HotCornerState::default();

        assert_eq!(state.update(Some(first), true, true), Some(first));
        assert_eq!(state.update(Some(second), true, true), None);
        assert_eq!(state.update(Some(second), true, true), Some(second));
    }

    fn armed_title_drag(win: WindowId, origin: super::ArmedDragOrigin) -> DragState {
        let mut interactions = DragState::default();
        interactions
            .arm_title_drag(super::ArmedDragParams {
                win,
                button: MouseButton::Left,
                source: InteractionSource::Pointer,
                origin,
                start: Point::new(300, 10),
                geometry: Rect::new(0, 0, 400, 300),
                restore_geometry: Rect::new(0, 0, 400, 300),
                was_focused: true,
                was_hidden: false,
                suppress_click_action: false,
            })
            .unwrap();
        interactions
    }

    #[test]
    fn title_reorder_transitions_from_armed_to_move_and_release() {
        let win = WindowId(5);
        let monitor = MonitorId::from_raw(2);
        let mut interactions = armed_title_drag(win, super::ArmedDragOrigin::BarTitle);

        interactions
            .begin_title_reorder(super::TitleReorderDrag::new(monitor))
            .unwrap();
        let (drag, reorder) = interactions.reordering_interaction().unwrap();
        assert_eq!(drag.win(), win);
        assert_eq!(reorder.monitor_id(), monitor);

        // An active drag or another capture cannot begin a reorder.
        assert!(
            interactions
                .begin_title_reorder(super::TitleReorderDrag::new(monitor))
                .is_err()
        );

        // Leaving the title strip converts the reorder into a move drag.
        interactions
            .activate_reordering_as_move(Point::new(320, 240), Rect::new(0, 0, 400, 300))
            .unwrap();
        assert!(interactions.reordering_interaction().is_none());
        let active = interactions.active_interaction().unwrap();
        assert_eq!(active.win(), win);
        assert_eq!(active.drag_type(), super::DragType::Move);

        // A fresh reorder ends cleanly on release.
        let mut interactions = armed_title_drag(win, super::ArmedDragOrigin::BarTitle);
        interactions
            .begin_title_reorder(super::TitleReorderDrag::new(monitor))
            .unwrap();
        assert!(interactions.finish_reordering().is_some());
        assert!(!interactions.has_capture());
    }

    #[test]
    fn title_reorder_requires_an_armed_capture() {
        let mut interactions = DragState::default();
        assert!(
            interactions
                .begin_title_reorder(super::TitleReorderDrag::new(MonitorId::from_raw(1)))
                .is_err()
        );

        // A client-origin armed drag arms fine but the caller never promotes
        // it to a reorder; the state machine itself stays in Armed.
        let interactions = armed_title_drag(WindowId(6), super::ArmedDragOrigin::Client);
        assert!(interactions.reordering_interaction().is_none());
        assert!(interactions.armed_interaction().is_some());
    }

    #[test]
    fn volume_drag_preserves_distance_across_compressed_motion() {
        let mut drag = SidebarVolumeDrag::new(
            MouseButton::Left,
            InteractionSource::Pointer,
            MonitorId::from_raw(3),
            500,
            30,
        );

        assert_eq!(drag.update(395), 3);
        assert_eq!(drag.update(381), 0);
        assert_eq!(drag.update(379), 1);
    }

    #[test]
    fn volume_drag_handles_direction_reversal_with_residual_distance() {
        let mut drag = SidebarVolumeDrag::new(
            MouseButton::Left,
            InteractionSource::Pointer,
            MonitorId::from_raw(3),
            500,
            30,
        );

        assert_eq!(drag.update(475), 0);
        assert_eq!(drag.update(510), 0);
        assert_eq!(drag.update(531), -1);
        assert_eq!(drag.update(469), 2);
    }

    #[test]
    fn sidebar_volume_lifecycle_rejects_overlap_and_wrong_button_release() {
        let drag = SidebarVolumeDrag::new(
            MouseButton::Left,
            InteractionSource::Pointer,
            MonitorId::from_raw(3),
            500,
            30,
        );
        let mut interactions = DragState::default();
        interactions
            .begin_overview_card(OverviewCardDrag::new(
                WindowId(1),
                MouseButton::Left,
                InteractionSource::Pointer,
                Point::new(0, 0),
                10,
            ))
            .unwrap();
        assert!(interactions.begin_sidebar_volume(drag).is_err());
        assert!(!interactions.sidebar_volume_active());

        interactions.cancel_capture();
        interactions.begin_sidebar_volume(drag).unwrap();
        assert!(!interactions.finish_sidebar_volume(MouseButton::Right));
        assert!(interactions.sidebar_volume_active());
        assert!(interactions.finish_sidebar_volume(MouseButton::Left));
        assert!(!interactions.sidebar_volume_active());
    }

    #[test]
    fn overview_card_gesture_resolves_tap_upward_drag_and_other_drag() {
        let win = WindowId(7);
        let gesture = || {
            OverviewCardDrag::new(
                win,
                MouseButton::Left,
                InteractionSource::Pointer,
                Point::new(500, 400),
                30,
            )
        };

        let mut tap = gesture();
        assert_eq!(tap.update(Point::new(510, 390)), None);
        assert_eq!(tap.action(), OverviewCardAction::Select(win));

        let mut upward = gesture();
        assert_eq!(upward.update(Point::new(520, 350)), Some(true));
        assert_eq!(upward.action(), OverviewCardAction::Close(win));
        assert_eq!(upward.update(Point::new(500, 390)), Some(false));
        assert_eq!(upward.action(), OverviewCardAction::Select(win));

        let mut horizontal = gesture();
        assert_eq!(horizontal.update(Point::new(550, 380)), None);
        assert_eq!(horizontal.action(), OverviewCardAction::Cancel);

        let mut downward = gesture();
        assert_eq!(downward.update(Point::new(500, 450)), None);
        assert_eq!(downward.action(), OverviewCardAction::Cancel);
    }

    #[test]
    fn overview_card_gesture_captures_its_complete_input_sequence() {
        let win = WindowId(9);
        let mut interactions = DragState::default();
        interactions
            .begin_overview_card(OverviewCardDrag::new(
                win,
                MouseButton::Left,
                InteractionSource::Touch(4),
                Point::new(100, 100),
                20,
            ))
            .unwrap();

        assert_eq!(interactions.captured_button(), Some(MouseButton::Left));
        assert_eq!(
            interactions.captured_source(),
            Some(InteractionSource::Touch(4))
        );
        assert!(interactions.has_capture());
        assert_eq!(
            interactions.finish_overview_card(MouseButton::Left),
            Some(OverviewCardAction::Select(win))
        );
        assert!(!interactions.has_capture());
    }
}

/// The pointer-owned interaction currently being offered before a click commits it.
///
/// This is the source of truth for hover offers; the cursor icon is a
/// side-effect, not the other way around.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum HoverOffer {
    #[default]
    None,
    /// Cursor is in the resize border zone of a floating window.
    Resize { win: WindowId, dir: ResizeDirection },
    /// Cursor is on the sidebar drag edge.
    Sidebar(SidebarTarget),
}

impl HoverOffer {
    /// Whether any hover interaction is offered (not [`HoverOffer::None`]).
    #[inline]
    pub fn is_active(self) -> bool {
        !matches!(self, HoverOffer::None)
    }

    #[inline]
    pub fn is_sidebar(self) -> bool {
        matches!(self, HoverOffer::Sidebar(_))
    }

    /// The resize target and direction when this is a border-resize offer.
    #[inline]
    pub fn resize_target(self) -> Option<(WindowId, ResizeDirection)> {
        match self {
            HoverOffer::Resize { win, dir } => Some((win, dir)),
            _ => None,
        }
    }
}

/// The single compositor-owned input sequence currently in progress.
///
/// Making capture mutually exclusive in the type system removes the former
/// priority ordering between several independent `Option` fields. Backends
/// can now decide whether to forward an event from this one source of truth.
#[derive(Debug, Clone)]
pub enum CapturedInteraction {
    Window(WindowDragState),
    Tag(TagDragState),
    SidebarVolume(SidebarVolumeDrag),
    BottomBar(BottomBarDrag),
    OverviewCard(OverviewCardDrag),
}

impl CapturedInteraction {
    pub fn button(&self) -> MouseButton {
        match self {
            Self::Window(state) => state.interaction().button(),
            Self::Tag(state) => state.button,
            Self::SidebarVolume(state) => state.button(),
            Self::BottomBar(state) => state.button(),
            Self::OverviewCard(state) => state.button(),
        }
    }

    pub fn source(&self) -> InteractionSource {
        match self {
            Self::Window(state) => state.interaction().source(),
            Self::Tag(state) => state.source,
            Self::SidebarVolume(state) => state.source(),
            Self::BottomBar(state) => state.source(),
            Self::OverviewCard(state) => state.source(),
        }
    }
}

/// Consolidated state for mouse/touch interactions.
#[derive(Debug, Clone, Default)]
pub struct DragState {
    capture: Option<CapturedInteraction>,
    pub hover_offer: HoverOffer,
}

impl DragState {
    pub fn capture(&self) -> Option<&CapturedInteraction> {
        self.capture.as_ref()
    }

    pub fn has_capture(&self) -> bool {
        self.capture.is_some()
    }

    pub fn active_interaction(&self) -> Option<&DragInteraction> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::Window(WindowDragState::Active(drag))) => Some(drag),
            _ => None,
        }
    }

    pub fn armed_interaction(&self) -> Option<&DragInteraction> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::Window(WindowDragState::Armed(drag))) => Some(drag),
            _ => None,
        }
    }

    pub fn reordering_interaction(&self) -> Option<(&DragInteraction, &TitleReorderDrag)> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::Window(WindowDragState::Reordering(drag, reorder))) => {
                Some((drag, reorder))
            }
            _ => None,
        }
    }

    /// Button whose complete press/motion/release sequence is WM-owned.
    pub fn captured_button(&self) -> Option<MouseButton> {
        self.capture.as_ref().map(CapturedInteraction::button)
    }

    pub fn captured_source(&self) -> Option<InteractionSource> {
        self.capture.as_ref().map(CapturedInteraction::source)
    }

    pub fn tag_drag(&self) -> Option<&TagDragState> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::Tag(drag)) => Some(drag),
            _ => None,
        }
    }

    pub fn tag_drag_mut(&mut self) -> Option<&mut TagDragState> {
        match self.capture.as_mut() {
            Some(CapturedInteraction::Tag(drag)) => Some(drag),
            _ => None,
        }
    }

    pub fn begin_tag_drag(&mut self, drag: TagDragState) -> Result<(), DragAlreadyActive> {
        self.begin_capture(CapturedInteraction::Tag(drag))
    }

    pub fn finish_tag_drag(&mut self, button: MouseButton) -> Option<TagDragState> {
        if !matches!(
            self.capture.as_ref(),
            Some(CapturedInteraction::Tag(drag)) if drag.button == button
        ) {
            return None;
        }
        match self.capture.take() {
            Some(CapturedInteraction::Tag(drag)) => Some(drag),
            _ => unreachable!(),
        }
    }

    pub fn begin_overview_card(&mut self, drag: OverviewCardDrag) -> Result<(), DragAlreadyActive> {
        self.begin_capture(CapturedInteraction::OverviewCard(drag))
    }

    pub fn update_overview_card(&mut self, root: Point) -> Option<bool> {
        match self.capture.as_mut() {
            Some(CapturedInteraction::OverviewCard(drag)) => drag.update(root),
            _ => None,
        }
    }

    pub fn finish_overview_card(&mut self, button: MouseButton) -> Option<OverviewCardAction> {
        if !matches!(
            self.capture.as_ref(),
            Some(CapturedInteraction::OverviewCard(drag)) if drag.button() == button
        ) {
            return None;
        }
        match self.capture.take() {
            Some(CapturedInteraction::OverviewCard(drag)) => Some(drag.action()),
            _ => unreachable!(),
        }
    }

    pub fn cancel_overview_card(&mut self) -> bool {
        if !matches!(self.capture, Some(CapturedInteraction::OverviewCard(_))) {
            return false;
        }
        self.capture = None;
        true
    }

    pub fn sidebar_volume_active(&self) -> bool {
        matches!(self.capture, Some(CapturedInteraction::SidebarVolume(_)))
    }

    pub fn sidebar_volume_button(&self) -> Option<MouseButton> {
        match self.capture {
            Some(CapturedInteraction::SidebarVolume(drag)) => Some(drag.button()),
            _ => None,
        }
    }

    pub fn sidebar_volume_monitor(&self) -> Option<MonitorId> {
        match self.capture {
            Some(CapturedInteraction::SidebarVolume(drag)) => Some(drag.monitor_id()),
            _ => None,
        }
    }

    pub fn begin_sidebar_volume(
        &mut self,
        drag: SidebarVolumeDrag,
    ) -> Result<(), DragAlreadyActive> {
        self.begin_capture(CapturedInteraction::SidebarVolume(drag))
    }

    pub fn update_sidebar_volume(&mut self, root_y: i32) -> Option<i32> {
        match self.capture.as_mut() {
            Some(CapturedInteraction::SidebarVolume(drag)) => Some(drag.update(root_y)),
            _ => None,
        }
    }

    pub fn finish_sidebar_volume(&mut self, button: MouseButton) -> bool {
        if self.sidebar_volume_button() != Some(button) {
            return false;
        }
        self.capture = None;
        true
    }

    pub fn cancel_sidebar_volume(&mut self) -> bool {
        if !self.sidebar_volume_active() {
            return false;
        }
        self.capture = None;
        true
    }

    pub fn bottom_bar_gesture_active(&self) -> bool {
        matches!(self.capture, Some(CapturedInteraction::BottomBar(_)))
    }

    pub fn bottom_bar_button(&self) -> Option<MouseButton> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::BottomBar(drag)) => Some(drag.button()),
            _ => None,
        }
    }

    pub fn bottom_bar_monitor(&self) -> Option<MonitorId> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::BottomBar(drag)) => Some(drag.monitor_id()),
            _ => None,
        }
    }

    pub fn bottom_bar_drag(&self) -> Option<&BottomBarDrag> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::BottomBar(drag)) => Some(drag),
            _ => None,
        }
    }

    pub fn begin_bottom_bar(&mut self, drag: BottomBarDrag) -> Result<(), DragAlreadyActive> {
        self.begin_capture(CapturedInteraction::BottomBar(drag))
    }

    pub fn update_bottom_bar(&mut self, root: Point) -> Option<SwipeDirection> {
        match self.capture.as_mut() {
            Some(CapturedInteraction::BottomBar(drag)) => drag.update(root),
            _ => None,
        }
    }

    pub fn finish_bottom_bar(&mut self, button: MouseButton) -> bool {
        if self.bottom_bar_button() != Some(button) {
            return false;
        }
        self.capture = None;
        true
    }

    pub fn cancel_bottom_bar(&mut self) -> bool {
        if !self.bottom_bar_gesture_active() {
            return false;
        }
        self.capture = None;
        true
    }

    pub fn begin_move(
        &mut self,
        win: WindowId,
        button: MouseButton,
        source: InteractionSource,
        start: Point,
        geo: Rect,
    ) -> Result<(), DragAlreadyActive> {
        self.begin_active(DragInteraction::immediate(
            win,
            button,
            source,
            DragOperation::Move,
            start,
            geo,
        ))
    }

    pub fn begin_resize(
        &mut self,
        win: WindowId,
        button: MouseButton,
        source: InteractionSource,
        dir: ResizeDirection,
        start: Point,
        geo: Rect,
    ) -> Result<(), DragAlreadyActive> {
        self.begin_resize_with_policy(ActiveResizeParams {
            win,
            button,
            source,
            direction: dir,
            start,
            geometry: geo,
            policy: ResizePolicy::Free,
        })
    }

    pub fn begin_resize_with_policy(
        &mut self,
        params: ActiveResizeParams,
    ) -> Result<(), DragAlreadyActive> {
        let mut drag = DragInteraction::immediate(
            params.win,
            params.button,
            params.source,
            DragOperation::Resize(params.direction),
            params.start,
            params.geometry,
        );
        drag.set_resize_policy(params.policy);
        self.begin_active(drag)
    }

    pub fn begin_tree_resize(&mut self, params: TreeResizeParams) -> Result<(), DragAlreadyActive> {
        self.begin_active(DragInteraction::immediate(
            params.win,
            params.button,
            params.source,
            DragOperation::TreeResize {
                direction: params.direction,
                origin: params.origin,
            },
            params.start,
            params.geometry,
        ))
    }

    fn begin_active(&mut self, drag: DragInteraction) -> Result<(), DragAlreadyActive> {
        self.begin_capture(CapturedInteraction::Window(WindowDragState::Active(drag)))
    }

    pub fn arm_title_drag(&mut self, params: ArmedDragParams) -> Result<(), DragAlreadyActive> {
        self.begin_capture(CapturedInteraction::Window(WindowDragState::Armed(
            DragInteraction::armed(params),
        )))
    }

    pub fn activate_armed(
        &mut self,
        drag_type: ArmedDragType,
        start: Point,
        geo: Rect,
    ) -> Result<(), DragNotArmed> {
        let mut drag = match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Armed(drag))) => drag,
            other => {
                self.capture = other;
                return Err(DragNotArmed);
            }
        };
        drag.activate_as(drag_type, start, geo);
        self.capture = Some(CapturedInteraction::Window(WindowDragState::Active(drag)));
        Ok(())
    }

    /// Promote an armed bar-title press to a live title-strip reorder.
    pub fn begin_title_reorder(&mut self, reorder: TitleReorderDrag) -> Result<(), DragNotArmed> {
        match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Armed(drag))) => {
                self.capture = Some(CapturedInteraction::Window(WindowDragState::Reordering(
                    drag, reorder,
                )));
                Ok(())
            }
            other => {
                self.capture = other;
                Err(DragNotArmed)
            }
        }
    }

    /// Convert a live title-strip reorder into an ordinary active move drag.
    pub fn activate_reordering_as_move(
        &mut self,
        start: Point,
        geo: Rect,
    ) -> Result<(), DragNotArmed> {
        let mut drag = match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Reordering(drag, _))) => drag,
            other => {
                self.capture = other;
                return Err(DragNotArmed);
            }
        };
        drag.activate_as(ArmedDragType::Move, start, geo);
        self.capture = Some(CapturedInteraction::Window(WindowDragState::Active(drag)));
        Ok(())
    }

    pub fn finish_reordering(&mut self) -> Option<DragInteraction> {
        match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Reordering(drag, _))) => Some(drag),
            other => {
                self.capture = other;
                None
            }
        }
    }

    pub fn record_interactive_motion(&mut self, point: Point) {
        if let Some(CapturedInteraction::Window(
            WindowDragState::Armed(drag)
            | WindowDragState::Reordering(drag, _)
            | WindowDragState::Active(drag),
        )) = self.capture.as_mut()
        {
            drag.record_motion(point)
        }
    }

    pub fn finish_active(&mut self, button: MouseButton) -> Option<DragInteraction> {
        if !self
            .active_interaction()
            .is_some_and(|drag| drag.button() == button)
        {
            return None;
        }
        match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Active(drag))) => Some(drag),
            _ => unreachable!(),
        }
    }

    pub fn finish_armed(&mut self) -> Option<DragInteraction> {
        match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Armed(drag))) => Some(drag),
            other => {
                self.capture = other;
                None
            }
        }
    }

    pub fn cancel_interactive(&mut self) -> Option<DragInteraction> {
        match self.capture.take() {
            Some(CapturedInteraction::Window(
                WindowDragState::Armed(drag)
                | WindowDragState::Reordering(drag, _)
                | WindowDragState::Active(drag),
            )) => Some(drag),
            other => {
                self.capture = other;
                None
            }
        }
    }

    pub fn cancel_capture(&mut self) -> Option<CapturedInteraction> {
        self.capture.take()
    }

    fn begin_capture(&mut self, capture: CapturedInteraction) -> Result<(), DragAlreadyActive> {
        if self.capture.is_some() {
            return Err(DragAlreadyActive);
        }
        self.capture = Some(capture);
        Ok(())
    }

    #[inline]
    pub fn set_hover_offer(&mut self, offer: HoverOffer) {
        self.hover_offer = offer;
    }

    /// Clears an active hover offer. Returns `true` if the state changed.
    pub fn clear_hover_offer(&mut self) -> bool {
        if !self.hover_offer.is_active() {
            return false;
        }
        self.hover_offer = HoverOffer::None;
        true
    }
}
