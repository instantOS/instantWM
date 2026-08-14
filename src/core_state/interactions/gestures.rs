use crate::actions::ButtonAction;

use super::*;

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
