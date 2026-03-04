//! Unified context for WM operations.
//!
//! WmCtx is the high-level orchestration layer. It delegates data management
//! to specialized Managers (MonitorManager, ClientManager) while providing
//! a simple, "fluent" API for the rest of the codebase.

use crate::backend::{BackendKind, BackendOps, BackendRef};
use crate::bar::BarState;
use crate::client::focus::FocusState;
use crate::globals::Globals;
use crate::types::{Client, Rect, WindowId};
use x11rb::rust_connection::RustConnection;

pub struct WmCtx<'a> {
    pub g: &'a mut Globals,
    pub backend: BackendRef<'a>,
    running: &'a mut bool,
    pub bar: &'a mut BarState,
    pub bar_painter: &'a mut crate::bar::wayland::WaylandBarPainter,
    pub focus: &'a mut FocusState,
}

impl<'a> WmCtx<'a> {
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

    // -------------------------------------------------------------------------
    // High-Level Action API (The DX Bridge)
    // -------------------------------------------------------------------------

    /// Get a client by window ID.
    pub fn client(&self, win: WindowId) -> Option<&Client> {
        self.g.clients.get(&win)
    }

    /// Get a client by window ID (mutable).
    pub fn client_mut(&mut self, win: WindowId) -> Option<&mut Client> {
        self.g.clients.get_mut(&win)
    }

    /// Get the currently selected client on the selected monitor.
    pub fn selected_client(&self) -> Option<WindowId> {
        self.g.selected_win()
    }

    /// Set the currently selected client on the selected monitor.
    pub fn set_selected_client(&mut self, win: Option<WindowId>) {
        self.g.selected_monitor_mut().sel = win;
    }

    /// Move/resize a client and sync with the backend.
    pub fn resize_client(&mut self, win: WindowId, rect: Rect) {
        if let Some(c) = self.g.clients.get_mut(&win) {
            c.old_geo = c.geo;
            c.geo = rect;
            self.backend.resize_window(win, rect);
        }
    }

    /// Update a client's border width in both memory and the window system.
    pub fn set_border(&mut self, win: WindowId, width: i32) {
        if let Some(c) = self.g.clients.get_mut(&win) {
            c.border_width = width;
            self.backend.set_border_width(win, width);
        }
    }

    /// Raise a window to the top of the stack.
    pub fn raise(&mut self, win: WindowId) {
        self.backend.raise_window(win);
    }

    /// Update stacking order.
    pub fn restack(&mut self, windows: &[WindowId]) {
        self.backend.restack(windows);
    }

    /// Flush all backend operations.
    pub fn flush(&mut self) {
        self.backend.flush();
    }

    // -------------------------------------------------------------------------
    // System Accessors
    // -------------------------------------------------------------------------

    pub fn x11_conn(&self) -> Option<crate::contexts::X11Conn<'_>> {
        self.backend
            .x11_conn()
            .map(|(conn, screen_num)| crate::contexts::X11Conn { conn, screen_num })
    }

    pub fn quit(&mut self) {
        *self.running = false;
    }

    pub fn backend_kind(&self) -> BackendKind {
        self.backend.kind()
    }
}

pub struct X11Conn<'a> {
    pub conn: &'a RustConnection,
    pub screen_num: usize,
}
