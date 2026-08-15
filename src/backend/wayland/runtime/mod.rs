//! Wayland backend runtime event loops.
//!
//! This module contains the main event loops for:
//! - Winit (nested) backend
//! - DRM/KMS (standalone) backend
//!
//! Shared per-tick logic lives in `engine`; each backend only adds
//! minimal backend-specific match arms.

pub mod bootstrap;
mod dispatch;
pub mod drm;
pub mod engine;
pub mod winit;
