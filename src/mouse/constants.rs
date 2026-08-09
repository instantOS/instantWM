//! Shared constants for the mouse-interaction subsystem.

// ── Window geometry limits ────────────────────────────────────────────────────

/// Minimum dimension (width or height) a window may be resized to via mouse.
pub const MIN_WINDOW_SIZE: i32 = 50;

/// Pixel band around a floating window's border that triggers resize-cursor activation.
pub const RESIZE_BORDER_ZONE: i32 = 30;

/// How many pixels the cursor must travel before a title-bar click is
/// promoted from a "click" to a "drag" in
/// [`crate::mouse::drag::handle_window_title_mouse`].
pub const DRAG_THRESHOLD: i32 = 5;

/// Width of the screen-edge zone (in pixels) that triggers an edge-snap
/// indicator during a window move interaction.
pub const OVERLAY_ZONE_WIDTH: i32 = 50;

/// Tolerance added around the monitor boundary when validating slop-selected
/// rectangles in [`crate::mouse::slop::is_valid_window_size`].
pub const SLOP_MARGIN: i32 = 40;
