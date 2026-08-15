//! Wayland backend initialization.
//!
//! The nested winit backend initializes directly in its runtime.  Only the
//! safety-sensitive DRM/GPU setup is kept in a separate module.

pub mod drm;
