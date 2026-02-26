//! Unified context for WM operations.
//!
//! Use a single context to avoid proliferation and keep dependencies explicit.

use crate::bar::BarState;
use crate::client::focus::FocusState;
use crate::globals::Globals;
use x11rb::rust_connection::RustConnection;

/// Unified WM context with globals and backend connection.
///
/// Today `WmCtx` always carries an X11 connection, but it is intentionally
/// structured so we can add a Wayland backend without rewriting all high-level
/// code at once.
pub struct WmCtx<'a> {
    pub g: &'a mut Globals,
    pub x11: X11Conn<'a>,
    running: &'a mut bool,
    pub bar: &'a mut BarState,
    pub focus: &'a mut FocusState,
}

/// A guaranteed X11 connection reference.
///
/// This exists so call-sites can stay mostly unchanged (`ctx.x11.conn`).
pub struct X11Conn<'a> {
    pub conn: &'a RustConnection,
    pub screen_num: usize,
}

/// Backend kind indicator.
///
/// This is a minimal hook so modules can start branching on backend capability
/// while we progressively move X11 details behind an abstraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    X11,
    Wayland,
}

impl<'a> WmCtx<'a> {
    /// Create a new unified context with a guaranteed X11 connection.
    pub fn new(
        g: &'a mut Globals,
        conn: &'a RustConnection,
        screen_num: usize,
        running: &'a mut bool,
        bar: &'a mut BarState,
        focus: &'a mut FocusState,
    ) -> Self {
        Self {
            g,
            x11: X11Conn { conn, screen_num },
            running,
            bar,
            focus,
        }
    }

    pub fn quit(&mut self) {
        *self.running = false;
    }

    #[inline]
    pub fn backend_kind(&self) -> BackendKind {
        BackendKind::X11
    }
}
