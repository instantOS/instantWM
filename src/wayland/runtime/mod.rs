//! Wayland backend runtime event loops.
//!
//! This module contains the main event loops for:
//! - Winit (nested) backend
//! - DRM/KMS (standalone) backend

pub mod drm;
pub mod winit;
