//! Winit (nested) backend initialization.
//!
//! The winit backend runs as a nested compositor inside an existing
//! Wayland or X11 session. Most initialization is shared via
//! `crate::wayland::common`.

// Re-export shared initialization helpers
pub use crate::wayland::common::sanitize_wayland_size;
