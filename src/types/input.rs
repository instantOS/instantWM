//! Input handling types.
//!
//! Types for mouse, keyboard, and gesture handling.

use std::str::FromStr;

use crate::types::{MonitorId, Point, Rect, Size, TagMask, WindowId};

/// Mouse buttons recognized by the window manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseButton {
    /// Left mouse button.
    #[default]
    Left,
    /// Middle mouse button.
    Middle,
    /// Right mouse button.
    Right,
    /// Scroll up.
    ScrollUp,
    /// Scroll down.
    ScrollDown,
}

impl MouseButton {
    /// Convert from an X11 button detail value.
    pub fn from_x11_detail(detail: u8) -> Option<Self> {
        match detail {
            1 => Some(Self::Left),
            2 => Some(Self::Middle),
            3 => Some(Self::Right),
            4 => Some(Self::ScrollUp),
            5 => Some(Self::ScrollDown),
            _ => None,
        }
    }

    /// Convert from a Wayland button code (Linux input event codes).
    pub fn from_wayland_code(code: u32) -> Option<Self> {
        match code {
            0x110 => Some(Self::Left),
            0x112 => Some(Self::Middle),
            0x111 => Some(Self::Right),
            _ => None,
        }
    }

    /// Convert to an X11 button detail value.
    pub fn to_x11_detail(self) -> u8 {
        match self {
            Self::Left => 1,
            Self::Middle => 2,
            Self::Right => 3,
            Self::ScrollUp => 4,
            Self::ScrollDown => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SidebarTarget {
    pub monitor_id: MonitorId,
    pub edge: EdgeDirection,
    pub rect: Rect,
}

/// Alternative cursor states for special operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AltCursor {
    /// No special cursor.
    #[default]
    Default,
    /// Move cursor.
    Move,
    /// Resize cursor with direction.
    Resize(ResizeDirection),
}

impl AltCursor {
    /// Convert cursor type to X11 cursor index (for cfg.cursors array).
    pub fn to_x11_index(self) -> usize {
        match self {
            AltCursor::Default => 0,
            AltCursor::Move => 2,
            AltCursor::Resize(dir) => match dir {
                ResizeDirection::TopLeft => 8,
                ResizeDirection::TopRight => 9,
                ResizeDirection::BottomLeft => 6,
                ResizeDirection::BottomRight => 7,
                ResizeDirection::Top => 5,
                ResizeDirection::Bottom => 5,
                ResizeDirection::Left => 4,
                ResizeDirection::Right => 4,
            },
        }
    }
}

/// Describes precisely what the mouse cursor is positioned over in the bar.
///
/// Non-bar targets are represented by [`crate::types::ButtonTarget`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BarPosition {
    /// The start-menu icon at the left edge of the bar.
    StartMenu,
    /// A tag indicator button. The inner value is the **0-based** tag index.
    Tag(usize),
    /// The layout symbol indicator (e.g. `[]=`).
    LayoutSymbol,
    /// The shutdown/power button (shown when no client is selected).
    ShutDown,
    /// The title cell of a specific client window.
    WinTitle(WindowId),
    /// The close button overlaying the left edge of the selected client's title.
    CloseButton(WindowId),
    /// The resize widget overlaying the right edge of the selected client's title.
    ResizeWidget(WindowId),
    /// The status-text / command strip on the right side of the bar.
    StatusText,
    /// A StatusNotifier tray item by index in the current tray model.
    SystrayItem(usize),
    /// An entry in the currently visible bar-native tray menu level.
    SystrayMenuItem(usize),
    /// An unoccupied area of the bar.
    #[default]
    Root,
}

impl BarPosition {
    /// Convert this position to a tag mask if it represents a tag button.
    ///
    /// Returns `None` for non-tag positions.
    pub fn to_tag_mask(&self) -> Option<TagMask> {
        match self {
            Self::Tag(idx) => TagMask::from_index(*idx),
            _ => None,
        }
    }

    /// Map this position to the `Gesture` used for hover highlighting.
    pub fn to_gesture(self) -> Gesture {
        match self {
            Self::StartMenu => Gesture::StartMenu,
            Self::Tag(idx) => Gesture::Tag(idx),
            Self::CloseButton(_) => Gesture::CloseButton,
            Self::WinTitle(w) => Gesture::WinTitle(w),
            _ => Gesture::None,
        }
    }
}

/// Describes which interactive bar region the cursor is hovering over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Gesture {
    /// No actionable hover target.
    #[default]
    None,
    /// Cursor is over a specific window-title cell in the bar.
    ///
    /// This is used to drive hover highlighting; unlike selection state, it can
    /// be empty (no title hovered) when the cursor leaves the bar.
    WinTitle(WindowId),
    /// Cursor is over a tag button (0-based tag index).
    Tag(usize),
    /// Cursor is over the close button.
    CloseButton,
    /// Cursor is over the start-menu icon.
    StartMenu,
}

impl Gesture {
    /// Construct a `Tag` gesture from a 0-based tag index.
    ///
    /// Returns `None` only if the index is unreasonably large (> 63).
    pub fn from_tag_index(tag_index: usize) -> Option<Self> {
        if tag_index < 64 {
            Some(Self::Tag(tag_index))
        } else {
            None
        }
    }

    /// Returns `true` if this gesture represents a tag hover.
    pub fn is_tag(self) -> bool {
        matches!(self, Self::Tag(_))
    }

    /// Returns the tag index if this is a `Tag` gesture, otherwise `None`.
    pub fn tag_index(self) -> Option<usize> {
        if let Self::Tag(idx) = self {
            Some(idx)
        } else {
            None
        }
    }
}

/// Snap position for window snapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SnapPosition {
    /// No snap.
    #[default]
    None,
    /// Snap to top edge.
    Top,
    /// Snap to top-right corner.
    TopRight,
    /// Snap to right edge.
    Right,
    /// Snap to bottom-right corner.
    BottomRight,
    /// Snap to bottom edge.
    Bottom,
    /// Snap to bottom-left corner.
    BottomLeft,
    /// Snap to left edge.
    Left,
    /// Snap to top-left corner.
    TopLeft,
    /// Maximized.
    Maximized,
}

impl SnapPosition {
    /// Navigate the snap graph: given a direction, return the next snap position.
    pub fn next(self, direction: Direction) -> Self {
        use Direction::*;
        match (self, direction) {
            // ── None ──────────────────────────────
            (Self::None, Up) => Self::Maximized,
            (Self::None, Right) => Self::Right,
            (Self::None, Down) => Self::Bottom,
            (Self::None, Left) => Self::Left,

            // ── Top ───────────────────────────────
            (Self::Top, Up) => Self::Maximized,
            (Self::Top, Right) => Self::TopRight,
            (Self::Top, Down) => Self::None,
            (Self::Top, Left) => Self::TopLeft,

            // ── TopRight ──────────────────────────
            (Self::TopRight, Up) => Self::TopRight,
            (Self::TopRight, Right) => Self::TopRight,
            (Self::TopRight, Down) => Self::Right,
            (Self::TopRight, Left) => Self::Top,

            // ── Right ─────────────────────────────
            (Self::Right, Up) => Self::TopRight,
            (Self::Right, Right) => Self::Right,
            (Self::Right, Down) => Self::BottomRight,
            (Self::Right, Left) => Self::None,

            // ── BottomRight ───────────────────────
            (Self::BottomRight, Up) => Self::Right,
            (Self::BottomRight, Right) => Self::BottomRight,
            (Self::BottomRight, Down) => Self::BottomRight,
            (Self::BottomRight, Left) => Self::Bottom,

            // ── Bottom ────────────────────────────
            (Self::Bottom, Up) => Self::None,
            (Self::Bottom, Right) => Self::BottomRight,
            (Self::Bottom, Down) => Self::Bottom,
            (Self::Bottom, Left) => Self::BottomLeft,

            // ── BottomLeft ────────────────────────
            (Self::BottomLeft, Up) => Self::Left,
            (Self::BottomLeft, Right) => Self::Bottom,
            (Self::BottomLeft, Down) => Self::BottomLeft,
            (Self::BottomLeft, Left) => Self::BottomLeft,

            // ── Left ──────────────────────────────
            (Self::Left, Up) => Self::TopLeft,
            (Self::Left, Right) => Self::None,
            (Self::Left, Down) => Self::BottomLeft,
            (Self::Left, Left) => Self::Left,

            // ── TopLeft ───────────────────────────
            (Self::TopLeft, Up) => Self::TopLeft,
            (Self::TopLeft, Right) => Self::Top,
            (Self::TopLeft, Down) => Self::Left,
            (Self::TopLeft, Left) => Self::Top,

            // ── Maximized ─────────────────────────
            (Self::Maximized, Up) => Self::Top,
            (Self::Maximized, Right) => Self::Right,
            (Self::Maximized, Down) => Self::None,
            (Self::Maximized, Left) => Self::Left,
        }
    }
}

/// Direction for window resize operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResizeDirection {
    /// Resize from top-left corner.
    TopLeft,
    /// Resize from top edge.
    Top,
    /// Resize from top-right corner.
    TopRight,
    /// Resize from right edge.
    Right,
    /// Resize from bottom-right corner.
    #[default]
    BottomRight,
    /// Resize from bottom edge.
    Bottom,
    /// Resize from bottom-left corner.
    BottomLeft,
    /// Resize from left edge.
    Left,
}

impl ResizeDirection {
    /// Get the cursor index for this resize direction.
    pub fn cursor_index(self) -> usize {
        match self {
            Self::TopLeft => 8,
            Self::Top => 4,
            Self::TopRight => 9,
            Self::Right => 5,
            Self::BottomRight => 7,
            Self::Bottom => 4,
            Self::BottomLeft => 6,
            Self::Left => 5,
        }
    }

    /// Get which edges are affected by this resize direction.
    ///
    /// Returns a tuple of (left, right, top, bottom) booleans.
    pub fn affected_edges(self) -> (bool, bool, bool, bool) {
        match self {
            Self::TopLeft => (true, false, true, false),
            Self::Top => (false, false, true, false),
            Self::TopRight => (false, true, true, false),
            Self::Right => (false, true, false, false),
            Self::BottomRight => (false, true, false, true),
            Self::Bottom => (false, false, false, true),
            Self::BottomLeft => (true, false, false, true),
            Self::Left => (true, false, false, false),
        }
    }

    /// Get the warp offset for this resize direction.
    ///
    /// Returns the cursor position relative to the window geometry.
    pub fn warp_offset(self, size: Size, border_width: i32) -> Point {
        let Size { w, h } = size;
        let bw = border_width;
        match self {
            Self::TopLeft => Point::new(-bw, -bw),
            Self::Top => Point::new((w + bw - 1) / 2, -bw),
            Self::TopRight => Point::new(w + bw - 1, -bw),
            Self::Right => Point::new(w + bw - 1, (h + bw - 1) / 2),
            Self::BottomRight => Point::new(w + bw - 1, h + bw - 1),
            Self::Bottom => Point::new((w + bw - 1) / 2, h + bw - 1),
            Self::BottomLeft => Point::new(-bw, h + bw - 1),
            Self::Left => Point::new(-bw, (h + bw - 1) / 2),
        }
    }

    /// Convert to Wayland cursor icon.
    pub fn to_wayland_icon(self) -> smithay::input::pointer::CursorIcon {
        match self {
            Self::TopLeft => smithay::input::pointer::CursorIcon::NwResize,
            Self::Top => smithay::input::pointer::CursorIcon::NResize,
            Self::TopRight => smithay::input::pointer::CursorIcon::NeResize,
            Self::Right => smithay::input::pointer::CursorIcon::EResize,
            Self::BottomRight => smithay::input::pointer::CursorIcon::SeResize,
            Self::Bottom => smithay::input::pointer::CursorIcon::SResize,
            Self::BottomLeft => smithay::input::pointer::CursorIcon::SwResize,
            Self::Left => smithay::input::pointer::CursorIcon::WResize,
        }
    }

    /// Determine the resize direction for a hit position within a window.
    pub fn from_hit(size: Size, hit: Point) -> Self {
        let Size { w, h } = size;
        let Point { x: hit_x, y: hit_y } = hit;
        if hit_y > h / 2 {
            if hit_x < w / 3 {
                if hit_y < 2 * h / 3 {
                    Self::Left
                } else {
                    Self::BottomLeft
                }
            } else if hit_x > 2 * w / 3 {
                if hit_y < 2 * h / 3 {
                    Self::Right
                } else {
                    Self::BottomRight
                }
            } else {
                Self::Bottom
            }
        } else if hit_x < w / 3 {
            if hit_y > h / 3 {
                Self::Left
            } else {
                Self::TopLeft
            }
        } else if hit_x > 2 * w / 3 {
            if hit_y > h / 3 {
                Self::Right
            } else {
                Self::TopRight
            }
        } else {
            Self::Top
        }
    }
}

/// The screen edge where an edge-anchored scratchpad slides in/out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeDirection {
    /// Slides down from the top edge (default).
    #[default]
    Top,
    /// Slides in from the right edge.
    Right,
    /// Slides up from the bottom edge.
    Bottom,
    /// Slides in from the left edge.
    Left,
}

impl EdgeDirection {
    /// Returns `true` for modes where the window is sized along the vertical axis.
    pub fn is_vertical(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }

    /// Lowercase name for serialization.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Right => "right",
            Self::Bottom => "bottom",
            Self::Left => "left",
        }
    }

    /// Parse from a case-insensitive string.
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "top" => Some(Self::Top),
            "right" => Some(Self::Right),
            "bottom" => Some(Self::Bottom),
            "left" => Some(Self::Left),
            _ => None,
        }
    }
}

/// Vertical axis (up / down).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerticalDirection {
    Up,
    Down,
}

/// Horizontal axis (left / right).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HorizontalDirection {
    Left,
    Right,
}

impl From<VerticalDirection> for Direction {
    fn from(v: VerticalDirection) -> Self {
        match v {
            VerticalDirection::Up => Self::Up,
            VerticalDirection::Down => Self::Down,
        }
    }
}

impl From<HorizontalDirection> for Direction {
    fn from(h: HorizontalDirection) -> Self {
        match h {
            HorizontalDirection::Left => Self::Left,
            HorizontalDirection::Right => Self::Right,
        }
    }
}

/// Cardinal direction for focus movement, floating move/resize, snap navigation, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Move up.
    Up,
    /// Move down.
    Down,
    /// Move left.
    Left,
    /// Move right.
    Right,
}

impl Direction {
    /// Get delta for movement as (dx, dy).
    pub fn move_delta(self, step: i32) -> (i32, i32) {
        match self {
            Self::Down => (0, step),
            Self::Up => (0, -step),
            Self::Right => (step, 0),
            Self::Left => (-step, 0),
        }
    }

    /// Get delta for resize as (dw, dh) - grow direction.
    pub fn resize_delta(self, step: i32) -> (i32, i32) {
        match self {
            Self::Down => (0, step),
            Self::Up => (0, -step),
            Self::Right => (step, 0),
            Self::Left => (-step, 0),
        }
    }

    /// `Some` if this is a vertical axis.
    pub fn as_vertical(self) -> Option<VerticalDirection> {
        match self {
            Self::Up => Some(VerticalDirection::Up),
            Self::Down => Some(VerticalDirection::Down),
            _ => None,
        }
    }

    /// `Some` if this is a horizontal axis.
    pub fn as_horizontal(self) -> Option<HorizontalDirection> {
        match self {
            Self::Left => Some(HorizontalDirection::Left),
            Self::Right => Some(HorizontalDirection::Right),
            _ => None,
        }
    }
}

/// Direction for stack-based focus movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StackDirection {
    /// Move to the next item in the stack.
    #[default]
    Next,
    /// Move to the previous item in the stack.
    Previous,
}

impl StackDirection {
    /// Parse a direction from a string name (aliases accepted).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::from_str(name).ok()
    }

    /// Returns true if this is the Next direction.
    pub fn is_forward(self) -> bool {
        matches!(self, Self::Next)
    }
}

impl FromStr for StackDirection {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "next" | "down" | "forward" => Ok(Self::Next),
            "prev" | "previous" | "up" | "backward" => Ok(Self::Previous),
            _ => Err(()),
        }
    }
}
