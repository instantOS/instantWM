//! Animation timing constants.

/// Default animation duration in milliseconds.
pub const DEFAULT_ANIMATION_MILLIS: u64 = 167;

/// Fixed vertical travel for a newly managed window's entrance transition.
pub const SPAWN_SLIDE_DISTANCE: i32 = 70;

/// Fixed horizontal travel for windows entering during an adjacent tag switch.
pub const TAG_SLIDE_DISTANCE: i32 = 70;

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

/// Duration in milliseconds for interactive window dragging animations.
pub const INTERACTIVE_DRAG_ANIMATION_MILLIS: u64 = 50;
