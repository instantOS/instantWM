//! Unified context for WM operations.
//!
//! Use a single context to avoid proliferation and keep dependencies explicit.

use crate::globals::{Globals, X11Conn};

/// Unified WM context with globals and X11 connection.
///
/// The X11 connection is guaranteed to be available after initialization.
/// If the connection is lost, the window manager cannot function and will panic.
pub struct WmCtx<'a> {
    pub g: &'a mut Globals,
    pub x11: X11Conn<'a>,
}

impl<'a> WmCtx<'a> {
    /// Create a new unified context with a guaranteed X11 connection.
    pub fn new(g: &'a mut Globals, x11: X11Conn<'a>) -> Self {
        Self { g, x11 }
    }
}
