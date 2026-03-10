//! Type definitions for instantWM.
//!
//! This module organizes types by domain/feature for better maintainability:
//! - `core` - Fundamental types and constants
//! - `geometry` - Rectangles, size hints, positioning
//! - `atoms` - X11 atom identifiers
//! - `color` - Color schemes and configurations
//! - `tag` - Tag system types (Tag, TagSet, TagLayouts)
//! - `client` - Client/window management types
//! - `monitor` - Monitor/screen types
//! - `input` - Input handling (mouse, keyboard, gestures)
//! - `rules` - Window rules and matching
//! - `commands` - Command action types
//! - `window` - Window system types
//!
//! For convenience, all types are re-exported at the module level.

// Core types and constants
pub mod core;
pub use core::*;

// Geometry types
pub mod geometry;
pub use geometry::*;

// X11 atoms
pub mod atoms;
pub use atoms::*;

// Color schemes
pub mod color;
pub use color::*;

// Tag system
pub mod tag;
pub use tag::*;

// Client/window types
pub mod client;
pub use client::*;

// Monitor types
pub mod monitor;
pub use monitor::*;

// Input types (mouse, keyboard, gestures)
pub mod input;
pub use input::*;

// Window rules
pub mod rules;
pub use rules::*;

// Command types
pub mod commands;
pub use commands::*;

// Window system types
pub mod window;
pub use window::*;

// Type-safe tag system (existing module)
pub mod tag_types;
pub use tag_types::{MonitorDirection, TagMask, TagSelection};
