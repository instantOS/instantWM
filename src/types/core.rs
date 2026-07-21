//! Core types and constants.
//!
//! Fundamental type aliases and constants used throughout the window manager.

/// X11 atom identifier (protocol type is CARDINAL / 32-bit).
pub type Atom = u32;

/// Client identifier type.
pub type ClientId = usize;

/// Stable, opaque identifier for a monitor.
///
/// A `MonitorId` is allocated by [`crate::monitor::MonitorManager`] when a
/// monitor is created and **never changes** for the lifetime of that monitor —
/// not across output hotplug, reordering, or reconfiguration. This is distinct
/// from a monitor's *spatial position* (its index in the display order), which
/// is queried separately via [`crate::monitor::MonitorManager::position_of`].
///
/// Identifiers are never reused, so a stale `MonitorId` (one whose monitor has
/// been removed) simply fails to resolve rather than silently referring to a
/// different monitor.
///
/// Construction is limited to the manager's internal allocator; all other code
/// obtains a `MonitorId` from a lookup or iteration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MonitorId(u64);

impl MonitorId {
    /// Stable numeric representation for diagnostics and IPC.
    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Construct a raw id from its internal representation.
    ///
    /// This is the sole construction path, intended only for the
    /// [`crate::monitor::MonitorManager`] allocator. Application code should
    /// never call this.
    #[inline]
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
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
