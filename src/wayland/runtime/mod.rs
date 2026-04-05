//! Wayland backend runtime event loops.
//!
//! This module contains the main event loops for:
//! - Winit (nested) backend
//! - DRM/KMS (standalone) backend
//!
//! Shared per-tick logic lives in `common`; each backend only adds
//! minimal backend-specific match arms.

pub mod common;
pub mod drm;
pub mod winit;
