//! X11 backend wrapper.
//!
//! instantWM uses `x11rb::RustConnection` directly throughout the codebase.
//! This wrapper exists to give us a stable place to hang backend-specific
//! functionality while still allowing existing call-sites to use the raw
//! connection.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, InputFocus, StackMode, Window};
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;

use crate::backend::{BackendKind, BackendOps};
use crate::types::{Rect, WindowId};

pub struct X11Backend {
    pub conn: RustConnection,
    pub screen_num: usize,
}

impl X11Backend {
    pub fn new(conn: RustConnection, screen_num: usize) -> Self {
        Self { conn, screen_num }
    }
}

/// Borrowed view of the X11 backend.
pub struct X11BackendRef<'a> {
    pub conn: &'a RustConnection,
    pub screen_num: usize,
}

impl<'a> X11BackendRef<'a> {
    pub fn new(conn: &'a RustConnection, screen_num: usize) -> Self {
        Self { conn, screen_num }
    }
}

impl BackendOps for X11Backend {
    fn kind(&self) -> BackendKind {
        BackendKind::X11
    }

    fn resize_window(&self, window: WindowId, rect: Rect) {
        let x11_win: Window = window.into();
        let width = rect.w.max(1) as u32;
        let height = rect.h.max(1) as u32;
        let _ = self.conn.configure_window(
            x11_win,
            &ConfigureWindowAux::new()
                .x(rect.x)
                .y(rect.y)
                .width(width)
                .height(height),
        );
    }

    fn raise_window(&self, window: WindowId) {
        let x11_win: Window = window.into();
        let _ = self.conn.configure_window(
            x11_win,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
    }

    fn restack(&self, windows: &[WindowId]) {
        for window in windows {
            self.raise_window(*window);
        }
    }

    fn set_focus(&self, window: WindowId) {
        let x11_win: Window = window.into();
        let _ = self
            .conn
            .set_input_focus(InputFocus::POINTER_ROOT, x11_win, CURRENT_TIME);
    }

    fn map_window(&self, window: WindowId) {
        let x11_win: Window = window.into();
        let _ = self.conn.map_window(x11_win);
    }

    fn unmap_window(&self, window: WindowId) {
        let x11_win: Window = window.into();
        let _ = self.conn.unmap_window(x11_win);
    }

    fn flush(&self) {
        let _ = self.conn.flush();
    }
}

impl BackendOps for X11BackendRef<'_> {
    fn kind(&self) -> BackendKind {
        BackendKind::X11
    }

    fn resize_window(&self, window: WindowId, rect: Rect) {
        let x11_win: Window = window.into();
        let width = rect.w.max(1) as u32;
        let height = rect.h.max(1) as u32;
        let _ = self.conn.configure_window(
            x11_win,
            &ConfigureWindowAux::new()
                .x(rect.x)
                .y(rect.y)
                .width(width)
                .height(height),
        );
    }

    fn raise_window(&self, window: WindowId) {
        let x11_win: Window = window.into();
        let _ = self.conn.configure_window(
            x11_win,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
    }

    fn restack(&self, windows: &[WindowId]) {
        for window in windows {
            self.raise_window(*window);
        }
    }

    fn set_focus(&self, window: WindowId) {
        let x11_win: Window = window.into();
        let _ = self
            .conn
            .set_input_focus(InputFocus::POINTER_ROOT, x11_win, CURRENT_TIME);
    }

    fn map_window(&self, window: WindowId) {
        let x11_win: Window = window.into();
        let _ = self.conn.map_window(x11_win);
    }

    fn unmap_window(&self, window: WindowId) {
        let x11_win: Window = window.into();
        let _ = self.conn.unmap_window(x11_win);
    }

    fn flush(&self) {
        let _ = self.conn.flush();
    }
}
