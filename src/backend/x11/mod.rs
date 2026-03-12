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

use crate::backend::BackendOps;
use crate::types::{Rect, WindowId};

pub mod bar;
pub mod client;
pub mod events;
pub mod lifecycle;
pub mod mouse;

pub use client::update_size_hints_x11;

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

    /// Create a borrowed reference to delegate operations to.
    fn as_ref(&self) -> X11BackendRef<'_> {
        X11BackendRef::new(&self.conn, self.screen_num)
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

/// RAII guard for X server grabs.
///
/// The WM uses two X11 connections (x11rb `RustConnection` + Xlib `Display*`
/// for bar drawing).  A server grab on one connection blocks requests from the
/// other.  If an `ungrab_server` sits in the write buffer while code on the
/// Xlib side calls `XSync`, the result is a deadlock.
///
/// This guard ensures the grab is always released **and flushed** when the
/// guard goes out of scope, making it impossible to forget the flush.
pub struct ServerGrab<'a> {
    conn: &'a RustConnection,
}

impl<'a> ServerGrab<'a> {
    /// Send `GrabServer` and return a guard that will ungrab+flush on drop.
    pub fn new(conn: &'a RustConnection) -> Self {
        let _ = conn.grab_server();
        Self { conn }
    }
}

impl Drop for ServerGrab<'_> {
    fn drop(&mut self) {
        let _ = self.conn.ungrab_server();
        let _ = self.conn.flush();
    }
}

impl BackendOps for X11Backend {
    fn resize_window(&self, window: WindowId, rect: Rect) {
        self.as_ref().resize_window(window, rect)
    }

    fn raise_window(&self, window: WindowId) {
        self.as_ref().raise_window(window)
    }

    fn restack(&self, windows: &[WindowId]) {
        self.as_ref().restack(windows)
    }

    fn set_focus(&self, window: WindowId) {
        self.as_ref().set_focus(window)
    }

    fn map_window(&self, window: WindowId) {
        self.as_ref().map_window(window)
    }

    fn unmap_window(&self, window: WindowId) {
        self.as_ref().unmap_window(window)
    }

    fn set_border_width(&self, window: WindowId, width: i32) {
        self.as_ref().set_border_width(window, width)
    }

    fn window_exists(&self, window: WindowId) -> bool {
        self.as_ref().window_exists(window)
    }

    fn flush(&self) {
        self.as_ref().flush()
    }

    fn pointer_location(&self) -> Option<(i32, i32)> {
        self.as_ref().pointer_location()
    }

    fn warp_pointer(&self, x: f64, y: f64) {
        self.as_ref().warp_pointer(x, y)
    }

    fn set_monitor_config(&self, name: &str, config: &crate::config::config_toml::MonitorConfig) {
        self.as_ref().set_monitor_config(name, config)
    }

    fn get_outputs(&self) -> Vec<crate::backend::BackendOutputInfo> {
        self.as_ref().get_outputs()
    }
}

impl BackendOps for X11BackendRef<'_> {
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

    fn pointer_location(&self) -> Option<(i32, i32)> {
        let root = self.conn.setup().roots[self.screen_num].root;
        let reply = self.conn.query_pointer(root).ok()?.reply().ok()?;
        Some((reply.root_x as i32, reply.root_y as i32))
    }

    fn warp_pointer(&self, x: f64, y: f64) {
        let root = self.conn.setup().roots[self.screen_num].root;
        let _ = self.conn.warp_pointer(
            CURRENT_TIME,
            root,
            0,
            0,
            0,
            0,
            x.round() as i16,
            y.round() as i16,
        );
        let _ = self.conn.flush();
    }

    fn set_monitor_config(&self, _name: &str, _config: &crate::config::config_toml::MonitorConfig) {
        // TODO: X11 XRandR support
    }

    fn get_outputs(&self) -> Vec<crate::backend::BackendOutputInfo> {
        let screen = &self.conn.setup().roots[self.screen_num];
        vec![crate::backend::BackendOutputInfo {
            name: "X11".to_owned(),
            rect: crate::types::Rect {
                x: 0,
                y: 0,
                w: screen.width_in_pixels as i32,
                h: screen.height_in_pixels as i32,
            },
        }]
    }
}
