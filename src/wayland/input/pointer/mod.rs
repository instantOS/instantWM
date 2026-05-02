//! Pointer input handling for Wayland compositor.
//!
//! This module handles all pointer-related input events:
//! - Motion (absolute and relative)
//! - Button clicks
//! - Axis/scroll events
//! - Drag operations (title drag, tag drag, resize drag)

pub mod axis;
pub mod button;
pub mod drag;
pub mod motion;
