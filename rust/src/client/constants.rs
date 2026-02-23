//! X11 window manager protocol constants used throughout client management.

// ---------------------------------------------------------------------------
// WM_STATE property values (ICCCM §4.1.3.1)
// ---------------------------------------------------------------------------

/// The window is visible on screen and interactable.
pub const WM_STATE_NORMAL: i32 = 1;

/// The window has been minimized / hidden (iconic state).
pub const WM_STATE_ICONIC: i32 = 3;

/// The window has been withdrawn (not managed / unmapped).
pub const WM_STATE_WITHDRAWN: i32 = 0;

// ---------------------------------------------------------------------------
// Motif Window Manager (MWM) hints
// These come from the Motif toolkit and are widely supported as a de-facto
// standard for controlling border/decoration hints.
// ---------------------------------------------------------------------------

/// Index into the MWM hints array that holds the flags field.
pub const MWM_HINTS_FLAGS_FIELD: usize = 0;

/// Index into the MWM hints array that holds the decorations field.
pub const MWM_HINTS_DECORATIONS_FIELD: usize = 2;

/// Flag bit indicating that the decorations field is present and valid.
pub const MWM_HINTS_DECORATIONS: u32 = 1 << 1;

/// Decoration flag: draw all decorations (border + title bar + buttons).
pub const MWM_DECOR_ALL: u32 = 1 << 0;

/// Decoration flag: draw the window border.
pub const MWM_DECOR_BORDER: u32 = 1 << 1;

/// Decoration flag: draw the title bar.
pub const MWM_DECOR_TITLE: u32 = 1 << 3;

// ---------------------------------------------------------------------------
// WM_HINTS flag bits (ICCCM §4.1.2.4)
// ---------------------------------------------------------------------------

/// WM_HINTS flag: the `input` field is present and meaningful.
pub const WM_HINTS_INPUT_HINT: u32 = 1;

/// WM_HINTS flag: the window has an urgency / attention request pending.
pub const WM_HINTS_URGENCY_HINT: u32 = 256;

// ---------------------------------------------------------------------------
// WM_NORMAL_HINTS / XSizeHints flag bits (ICCCM §4.1.2.3)
// ---------------------------------------------------------------------------

/// XSizeHints flag: `base_width` / `base_height` fields are present.
pub const SIZE_HINTS_P_BASE_SIZE: u32 = 8;

/// XSizeHints flag: `min_width` / `min_height` fields are present.
pub const SIZE_HINTS_P_MIN_SIZE: u32 = 16;

/// XSizeHints flag: `max_width` / `max_height` fields are present.
pub const SIZE_HINTS_P_MAX_SIZE: u32 = 32;

/// XSizeHints flag: `width_inc` / `height_inc` resize-increment fields are present.
pub const SIZE_HINTS_P_RESIZE_INC: u32 = 64;

/// XSizeHints flag: `min_aspect` / `max_aspect` fields are present.
pub const SIZE_HINTS_P_ASPECT: u32 = 128;

// ---------------------------------------------------------------------------
// Misc
// ---------------------------------------------------------------------------

/// Placeholder window title used when the real title cannot be read.
pub const BROKEN: &str = "broken";
