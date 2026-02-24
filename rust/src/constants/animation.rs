/// Events in queue above which animations are skipped entirely
pub const QUEUE_SKIP_THRESHOLD: i32 = 100;

/// Events in queue above which animation frames are halved
pub const QUEUE_REDUCE_THRESHOLD: i32 = 50;

/// Minimum pixel distance for movement to be considered significant
pub const DISTANCE_THRESHOLD: i32 = 10;

/// Monitor width margin for animation decision logic
pub const MONITOR_WIDTH_THRESHOLD: i32 = 50;

/// Sleep duration in microseconds between animation frames
pub const FRAME_SLEEP_MICROS: u64 = 15000;

/// Maximum tag number for animation scroll limits
pub const MAX_TAG_NUMBER: u32 = 20;

/// Number of frames for overlay animations
pub const OVERLAY_ANIMATION_FRAMES: i32 = 15;
