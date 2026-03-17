//! Wayland compositor support.
//!
//! This module contains all Wayland-related functionality organized into
//! submodules by concern:
//!
//! - `init`: Initialization code for different backends (winit, DRM)
//! - `runtime`: Runtime event handling and main loops
//! - `input`: Input event handlers (keyboard, pointer)
//! - `render`: Rendering (borders, bar, scene composition)
//! - `common`: Shared utilities between all Wayland backends

pub mod common;
pub mod init;
pub mod input;
pub mod render;
pub mod runtime;

// Re-export commonly used items
pub use render::borders::render_border_elements;
