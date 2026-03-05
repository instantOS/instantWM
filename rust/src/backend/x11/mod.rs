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

pub mod bar;
pub mod client;
pub mod events;
pub mod lifecycle;
pub mod mouse;

/// Log X11 errors instead of silently ignoring them.
#[inline]
pub fn log_x11_error<T>(result: Result<T, x11rb::errors::ConnectionError>, operation: &str) {
    if let Err(e) = result {
        log::warn!("X11 operation '{}' failed: {}", operation, e);
    }
}

/// Log X11 protocol errors (have a different error type).
#[inline]
pub fn log_x11_protocol_error<T>(result: Result<T, x11rb::errors::ReplyError>, operation: &str) {
    if let Err(e) = result {
        log::warn!("X11 protocol operation '{}' failed: {}", operation, e);
    }
}

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

    fn set_border_width(&self, window: WindowId, width: i32) {
        let x11_win: Window = window.into();
        let _ = self.conn.configure_window(
            x11_win,
            &ConfigureWindowAux::new().border_width(width.max(0) as u32),
        );
    }

    fn window_exists(&self, window: WindowId) -> bool {
        let x11_win: Window = window.into();
        self.conn.get_window_attributes(x11_win).is_ok()
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

    fn set_border_width(&self, window: WindowId, width: i32) {
        let x11_win: Window = window.into();
        let _ = self.conn.configure_window(
            x11_win,
            &ConfigureWindowAux::new().border_width(width.max(0) as u32),
        );
    }

    fn window_exists(&self, window: WindowId) -> bool {
        let x11_win: Window = window.into();
        self.conn.get_window_attributes(x11_win).is_ok()
    }

    fn flush(&self) {
        let _ = self.conn.flush();
    }
}
