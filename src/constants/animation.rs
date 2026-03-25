//! Animation timing and frame count constants.

/// Default animation frame count for smooth animations.
pub const DEFAULT_FRAME_COUNT: i32 = 7;

/// Fast animation frame count when many clients are present.
pub const FAST_FRAME_COUNT: i32 = 4;

/// Threshold for switching to fast animations (number of clients).
pub const FAST_ANIM_THRESHOLD: usize = 5;

/// Mouse event loop rate (events per second).
pub const MOUSE_EVENT_RATE: u32 = 60;

/// Border width multiplier for calculating total window dimensions.
pub const BORDER_MULTIPLIER: i32 = 2;

/// Number of concurrent X11 window animations that still get full animation.
pub const X11_ANIM_FULL_THRESHOLD: usize = 4;

/// Number of concurrent X11 window animations after which new ones are shortened.
pub const X11_ANIM_REDUCE_THRESHOLD: usize = 8;

/// Minimum distance threshold for animation to be considered moving.
pub const DISTANCE_THRESHOLD: i32 = 5;

/// Monitor width threshold for animation behavior.
pub const MONITOR_WIDTH_THRESHOLD: i32 = 100;

/// Frame sleep duration in microseconds.
pub const FRAME_SLEEP_MICROS: u64 = 16667;

/// Maximum tag number for view scrolling.
pub const MAX_TAG_NUMBER: u32 = 9;

/// Number of frames for overlay window animations.
pub const OVERLAY_ANIMATION_FRAMES: i32 = 10;
