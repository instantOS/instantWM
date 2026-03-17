//! Wayland backend initialization.
//!
//! This module contains backend-specific initialization code for:
//! - Winit (nested) backend
//! - DRM/KMS (standalone) backend

pub mod drm;
pub mod winit;
