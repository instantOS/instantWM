//! Unified context for WM operations.
//!
//! Use a single context to avoid proliferation and keep dependencies explicit.

use crate::globals::{Globals, X11Connection};

/// Unified WM context with globals and X11 connection.
pub struct WmCtx<'a> {
    pub g: &'a mut Globals,
    pub x11: &'a X11Connection,
}

impl<'a> WmCtx<'a> {
    /// Create a new unified context.
    pub fn new(g: &'a mut Globals, x11: &'a X11Connection) -> Self {
        Self { g, x11 }
    }
}
