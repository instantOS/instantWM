//! Wayland compositor support.
//!
//! This module contains all Wayland-related functionality organized into
//! submodules by concern:
//!
//! - `render`: Rendering (borders, bar, scene composition)
//! - `init`: Initialization code for different backends
//! - `runtime`: Runtime event handling (frame callbacks, socket management)

pub mod render;

// Re-export commonly used items
pub use render::borders::render_border_elements;
