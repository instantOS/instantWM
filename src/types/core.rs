//! Core types and constants.
//!
//! Fundamental type aliases and constants used throughout the window manager.

/// X11 atom identifier (protocol type is CARDINAL / 32-bit).
pub type Atom = u32;

/// Client identifier type.
pub type ClientId = usize;

/// Monitor identifier type.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct MonitorId(pub usize);

impl MonitorId {
    #[inline]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    #[inline]
    pub const fn index(self) -> usize {
        self.0
    }
}

impl From<usize> for MonitorId {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<MonitorId> for usize {
    fn from(value: MonitorId) -> Self {
        value.0
    }
}

// =============================================================================
// Tag Constants
// =============================================================================

/// Maximum number of tags supported.
pub const MAX_TAGS: usize = 21;

/// Tag index used for scratchpad windows.
pub const SCRATCHPAD_TAG: usize = 20;

/// Bitmask for the scratchpad tag.
pub const SCRATCHPAD_MASK: u32 = 1 << SCRATCHPAD_TAG;

/// Maximum length of scratchpad names.
pub const SCRATCHPAD_NAME_LEN: usize = 64;

// =============================================================================
// Mouse/Button Constants
// =============================================================================

/// Button mask for button events.
pub const BUTTONMASK: u32 = 1 << 2 | 1 << 3;

/// Mouse event mask (includes button and motion).
pub const MOUSEMASK: u32 = BUTTONMASK | 1 << 6;

// =============================================================================
// UI Dimension Constants
// =============================================================================

/// Width of window close button in pixels.
pub const CLOSE_BUTTON_WIDTH: i32 = 20;

/// Height of window close button in pixels.
pub const CLOSE_BUTTON_HEIGHT: i32 = 16;

/// Detail/padding inside close button.
pub const CLOSE_BUTTON_DETAIL: i32 = 4;

/// Hit area width for close button (larger than visual for usability).
pub const CLOSE_BUTTON_HIT_WIDTH: i32 = 32;

/// Width of resize widget in pixels.
pub const RESIZE_WIDGET_WIDTH: i32 = 30;

/// Width of sidebar in pixels.
pub const SIDEBAR_WIDTH: i32 = 50;

// =============================================================================
// Overlay Constants
// =============================================================================

/// Width of overlay activation zone in pixels.
pub const OVERLAY_ACTIVATION_ZONE: i32 = 20;

/// X keep zone for overlay.
pub const OVERLAY_KEEP_ZONE_X: i32 = 40;

/// Y keep zone for overlay.
pub const OVERLAY_KEEP_ZONE_Y: i32 = 30;
