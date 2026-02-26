//! Backend abstraction.
//!
//! This module supports multiple window-system backends:
//! - **X11** (always available) — the original `x11rb`-based backend.
//! - **Wayland** (feature-gated behind `wayland_backend`) — a Smithay-based
//!   Wayland compositor backend.

pub mod wayland;
pub mod x11;
