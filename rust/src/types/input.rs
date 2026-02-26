//! Input handling types.
//!
//! Types for mouse, keyboard, and gesture handling.

/// Mouse cursor types used by the window manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    /// Normal/default cursor.
    Normal,
    /// Resize cursor.
    Resize,
    /// Move cursor.
    Move,
    /// Click/hand cursor.
    Click,
    /// Horizontal resize cursor.
    Hor,
    /// Vertical resize cursor.
    Vert,
    /// Top-left resize cursor.
    TL,
    /// Top-right resize cursor.
    TR,
    /// Bottom-left resize cursor.
    BL,
    /// Bottom-right resize cursor.
    BR,
}

/// Mouse buttons recognized by the window manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    /// Left mouse button.
    Left = 1,
    /// Middle mouse button.
    Middle = 2,
    /// Right mouse button.
    Right = 3,
    /// Scroll up.
    ScrollUp = 4,
    /// Scroll down.
    ScrollDown = 5,
}

impl MouseButton {
    /// Convert from a u8 value.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Left),
            2 => Some(Self::Middle),
            3 => Some(Self::Right),
            4 => Some(Self::ScrollUp),
            5 => Some(Self::ScrollDown),
            _ => None,
        }
    }

    /// Convert to a u8 value.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Alternative cursor states for special operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AltCursor {
    /// No special cursor.
    #[default]
    None,
    /// Resize cursor.
    Resize,
    /// Sidebar cursor.
    Sidebar,
}

/// Describes precisely what the mouse cursor is positioned over in the bar,
/// or what area of the screen was clicked outside the bar.
///
/// This is the single source of truth for all click dispatch. `Button` actions
/// receive the full `BarPosition` so they can access the exact target (e.g.
/// which tag index, which window) without any separate lookup.
///
/// For clicks outside the bar (`ClientWin`, `SideBar`), the non-bar variants
/// are used and bar-specific data (tag index, window) is not applicable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BarPosition {
    /// The start-menu icon at the left edge of the bar.
    StartMenu,
    /// A tag indicator button. The inner value is the **0-based** tag index.
    Tag(usize),
    /// The layout symbol indicator (e.g. `[]=`).
    LtSymbol,
    /// The shutdown/power button (shown when no client is selected).
    ShutDown,
    /// The title cell of a specific client window.
    WinTitle(x11rb::protocol::xproto::Window),
    /// The close button overlaying the left edge of the selected client's title.
    CloseButton(x11rb::protocol::xproto::Window),
    /// The resize widget overlaying the right edge of the selected client's title.
    ResizeWidget(x11rb::protocol::xproto::Window),
    /// The status-text / command strip on the right side of the bar.
    StatusText,
    /// A client window (outside the bar entirely).
    ClientWin,
    /// The sidebar activation zone.
    SideBar,
    /// An unoccupied area of the bar or the root window.
    #[default]
    Root,
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
    WinTitle(x11rb::protocol::xproto::Window),
    /// Cursor is over a tag button (0-based tag index).
    Tag(usize),
    /// Cursor is over the overlay activation zone.
    Overlay,
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
    #[allow(dead_code)]
    pub fn is_tag(self) -> bool {
        matches!(self, Self::Tag(_))
    }

    /// Returns the tag index if this is a `Tag` gesture, otherwise `None`.
    #[allow(dead_code)]
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

/// Direction for window resize operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Returns the (x, y) offset based on window dimensions.
    pub fn warp_offset(self, w: i32, h: i32, bw: i32) -> (i32, i32) {
        match self {
            Self::TopLeft => (-bw, -bw),
            Self::Top => ((w + bw - 1) / 2, -bw),
            Self::TopRight => (w + bw - 1, -bw),
            Self::Right => (w + bw - 1, (h + bw - 1) / 2),
            Self::BottomRight => (w + bw - 1, h + bw - 1),
            Self::Bottom => ((w + bw - 1) / 2, h + bw - 1),
            Self::BottomLeft => (-bw, h + bw - 1),
            Self::Left => (-bw, (h + bw - 1) / 2),
        }
    }
}

/// Determine resize direction from hit position within a window.
pub fn get_resize_direction(w: i32, h: i32, hit_x: i32, hit_y: i32) -> ResizeDirection {
    if hit_y > h / 2 {
        if hit_x < w / 3 {
            if hit_y < 2 * h / 3 {
                ResizeDirection::Left
            } else {
                ResizeDirection::BottomLeft
            }
        } else if hit_x > 2 * w / 3 {
            if hit_y < 2 * h / 3 {
                ResizeDirection::Right
            } else {
                ResizeDirection::BottomRight
            }
        } else {
            ResizeDirection::Bottom
        }
    } else if hit_x < w / 3 {
        if hit_y > h / 3 {
            ResizeDirection::Left
        } else {
            ResizeDirection::TopLeft
        }
    } else if hit_x > 2 * w / 3 {
        if hit_y > h / 3 {
            ResizeDirection::Right
        } else {
            ResizeDirection::TopRight
        }
    } else {
        ResizeDirection::Top
    }
}

/// The side of the screen from which the overlay window slides in/out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverlayMode {
    /// Overlay slides down from the top edge (default).
    #[default]
    Top,
    /// Overlay slides in from the right edge.
    Right,
    /// Overlay slides up from the bottom edge.
    Bottom,
    /// Overlay slides in from the left edge.
    Left,
}

impl OverlayMode {
    /// Convert a raw `i32` to an `OverlayMode`.
    ///
    /// Returns `None` for any value outside `0..=3`.
    pub fn from_i32(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::Top),
            1 => Some(Self::Right),
            2 => Some(Self::Bottom),
            3 => Some(Self::Left),
            _ => None,
        }
    }

    /// Return the canonical `i32` representation of this mode.
    pub fn to_i32(self) -> i32 {
        match self {
            Self::Top => 0,
            Self::Right => 1,
            Self::Bottom => 2,
            Self::Left => 3,
        }
    }

    /// Returns `true` for modes where the overlay is sized along the vertical axis.
    pub fn is_vertical(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }
}

/// Direction for focus movement and keyboard-driven operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Convert from integer (for backward compatibility).
    pub fn from_i32(i: i32) -> Option<Self> {
        match i {
            0 => Some(Self::Down),
            1 => Some(Self::Up),
            2 => Some(Self::Right),
            3 => Some(Self::Left),
            _ => None,
        }
    }

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
    /// Returns true if this is the Next direction.
    pub fn is_forward(self) -> bool {
        matches!(self, Self::Next)
    }

    /// Parse from i32 (for command compatibility).
    /// Positive = Next, negative/zero = Previous.
    pub fn from_i32(v: i32) -> Self {
        if v > 0 {
            Self::Next
        } else {
            Self::Previous
        }
    }
}
