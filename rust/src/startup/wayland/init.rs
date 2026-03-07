//! Winit-backend startup helpers.
//!
//! Everything that is shared with the DRM backend lives in
//! `crate::startup::common_wayland`.

// Re-export the shared helpers so that `wayland.rs` can still import them
// from `self::init` without changing its import paths.
pub(super) use crate::startup::common_wayland::sanitize_wayland_size;
