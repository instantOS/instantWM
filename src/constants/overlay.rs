//! Overlay window positioning constants.
//!
//! These constants define the margins and insets used when positioning
//! overlay windows on screen edges.

/// Horizontal margin from screen edge for overlay windows.
///
/// Used to offset the overlay window's x position from the left or right
/// screen edge, creating a small gap for visual separation.
pub const OVERLAY_MARGIN_X: i32 = 20;

/// Vertical margin from screen edge for vertical overlay windows.
///
/// Used as the y offset when positioning overlays on the left or right
/// screen edges (OverlayMode::Left, OverlayMode::Right).
pub const OVERLAY_MARGIN_Y: i32 = 40;

/// Total horizontal inset for horizontal overlay windows.
///
/// The total width reduction applied to horizontal overlays (top/bottom),
/// accounting for margins on both sides (2 * OVERLAY_MARGIN_X).
pub const OVERLAY_INSET_X: i32 = 40;

/// Total vertical inset for vertical overlay windows.
///
/// The total height reduction applied to vertical overlays (left/right),
/// accounting for margins at top and bottom.
pub const OVERLAY_INSET_Y: i32 = 80;
