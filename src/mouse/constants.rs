#![allow(dead_code)]
//! Shared constants for the mouse-interaction subsystem.

// ── Window geometry limits ────────────────────────────────────────────────────

/// Minimum dimension (width or height) a window may be resized to via mouse.
pub const MIN_WINDOW_SIZE: i32 = 50;

/// Pixel band around a floating window's border that triggers resize-cursor
/// activation in [`crate::mouse::resize::is_in_resize_border`].
pub const RESIZE_BORDER_ZONE: i32 = 30;

/// How many pixels the cursor must travel before a title-bar click is
/// promoted from a "click" to a "drag" in
/// [`crate::mouse::drag::window_title_mouse_handler`].
pub const DRAG_THRESHOLD: i32 = 5;

/// If a window's edge is within this many pixels of the monitor edge when
/// `move_mouse` starts, we assume it is "maximized" and restore the saved
/// float geometry instead of moving it.
pub const MAX_UNMAXIMIZE_OFFSET: i32 = 100;

/// Width of the screen-edge zone (in pixels) that triggers an edge-snap
/// indicator during [`crate::mouse::drag::move_mouse`].
pub const OVERLAY_ZONE_WIDTH: i32 = 50;

/// Tolerance added around the monitor boundary when validating slop-selected
/// rectangles in [`crate::mouse::slop::is_valid_window_size`].
pub const SLOP_MARGIN: i32 = 40;

// ── Refresh-rate throttling ───────────────────────────────────────────────────

/// High-frequency motion-event cap (events per second) used when
/// `globals.doubledraw` is enabled.
pub const REFRESH_RATE_HI: u32 = 240;

/// Default motion-event cap (events per second).
pub const REFRESH_RATE_LO: u32 = 120;

// ── X11 keycodes ─────────────────────────────────────────────────────────────

/// X11 hardware keycode for the Escape key, used to abort hover-resize.
pub const KEYCODE_ESCAPE: u8 = 9;

// ── Resize-direction indices ──────────────────────────────────────────────────
//
// These index into whichever directional logic needs to identify which corner
// or edge of a window is being dragged.

pub const RESIZE_DIR_TOP_LEFT: i32 = 0;
pub const RESIZE_DIR_TOP: i32 = 1;
pub const RESIZE_DIR_TOP_RIGHT: i32 = 2;
pub const RESIZE_DIR_RIGHT: i32 = 3;
pub const RESIZE_DIR_BOTTOM_RIGHT: i32 = 4;
pub const RESIZE_DIR_BOTTOM: i32 = 5;
pub const RESIZE_DIR_BOTTOM_LEFT: i32 = 6;
pub const RESIZE_DIR_LEFT: i32 = 7;
