//! Wayland compositor rendering.
//!
//! This module contains rendering code for:
//! - Winit (nested) backend
//! - DRM/KMS (standalone) backend
//! - Window borders (shared)

pub mod borders;
pub mod drm;
pub mod winit;
