//! Animation timing constants.

/// Default animation duration in milliseconds.
pub const DEFAULT_ANIMATION_MILLIS: u64 = 167;

/// Duration in milliseconds for small keyboard-driven floating moves.
pub const FLOAT_MOVE_ANIMATION_MILLIS: u64 = 117;

/// Duration in milliseconds for hide/minimize and fullscreen expansion transitions.
pub const EMPHASIZED_ANIMATION_MILLIS: u64 = 233;

/// Duration in milliseconds for decorative show/unhide slide-ins.
pub const DECORATIVE_SHOW_ANIMATION_MILLIS: u64 = 333;

/// Border width multiplier for calculating total window dimensions.
pub const BORDER_MULTIPLIER: i32 = 2;

/// Minimum distance threshold for animation to be considered moving.
pub const DISTANCE_THRESHOLD: i32 = 5;

/// Default Wayland animation duration in milliseconds.
pub const WAYLAND_DEFAULT_ANIMATION_MILLIS: u64 = 129;
