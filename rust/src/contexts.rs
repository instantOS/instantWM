//! Unified context for WM operations.
//!
//! Use a single context to avoid proliferation and keep dependencies explicit.

use crate::backend::{BackendKind, BackendOps, BackendRef};
use crate::bar::BarState;
use crate::client::focus::FocusState;
use crate::globals::Globals;
use x11rb::rust_connection::RustConnection;

/// Unified WM context with globals and backend connection.
///
/// `WmCtx` keeps backend access explicit while allowing X11-only code paths to
/// opt-in to X11 connections when available.
pub struct WmCtx<'a> {
    pub g: &'a mut Globals,
    pub backend: BackendRef<'a>,
    running: &'a mut bool,
    pub bar: &'a mut BarState,
    pub bar_painter: &'a mut crate::bar::wayland::WaylandBarPainter,
    pub focus: &'a mut FocusState,
}

/// An X11 connection reference, available only for X11 backends.
pub struct X11Conn<'a> {
    pub conn: &'a RustConnection,
    pub screen_num: usize,
}

impl<'a> WmCtx<'a> {
    /// Create a new unified context with a backend reference.
    pub fn new(
        g: &'a mut Globals,
        backend: BackendRef<'a>,
        running: &'a mut bool,
        bar: &'a mut BarState,
        bar_painter: &'a mut crate::bar::wayland::WaylandBarPainter,
        focus: &'a mut FocusState,
    ) -> Self {
        Self {
            g,
            backend,
            running,
            bar,
            bar_painter,
            focus,
        }
    }

    pub fn x11_conn(&self) -> Option<X11Conn<'_>> {
        self.backend
            .x11_conn()
            .map(|(conn, screen_num)| X11Conn { conn, screen_num })
    }

    pub fn with_x11_conn<T>(&self, f: impl FnOnce(&RustConnection, usize) -> T) -> Option<T> {
        self.backend
            .x11_conn()
            .map(|(conn, screen_num)| f(conn, screen_num))
    }

    pub fn quit(&mut self) {
        *self.running = false;
    }

    #[inline]
    pub fn backend_kind(&self) -> BackendKind {
        self.backend.kind()
    }
}
