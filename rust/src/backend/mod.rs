//! Backend abstraction.
//!
//! This module is a small scaffold to allow supporting multiple compositor/
//! window-system backends (X11 today; Wayland later).

pub mod wayland;
pub mod x11;
