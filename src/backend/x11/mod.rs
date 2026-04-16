//! X11 backend wrapper.
//!
//! instantWM uses `x11rb::RustConnection` directly throughout the codebase.
//! This wrapper exists to give us a stable place to hang backend-specific
//! functionality while still allowing existing call-sites to use the raw
//! connection.

use libc::c_void;
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, InputFocus, StackMode, Window};
use x11rb::rust_connection::RustConnection;

use crate::backend::BackendOps;
use crate::backend::x11::draw::{Cursor, Drw};
use crate::types::Atom;
use crate::types::atoms::{NetAtoms, WmAtoms, XAtoms};
use crate::types::color::{BorderScheme, StatusScheme};
use crate::types::{Rect, WindowId};

#[derive(Clone, Copy)]
pub struct XlibDisplay(pub *mut c_void);
unsafe impl Send for XlibDisplay {}
unsafe impl Sync for XlibDisplay {}

/// X11-specific runtime configuration.
/// These fields are only meaningful on X11 and are left as defaults/zero on Wayland/DRM.
#[derive(Clone)]
pub struct X11RuntimeConfig {
    pub wmatom: WmAtoms,
    pub netatom: NetAtoms,
    pub xatom: XAtoms,
    pub motifatom: Atom,
    pub numlockmask: u32,
    pub root: Window,
    /// The small 1×1 window for _NET_SUPPORTING_WM_CHECK (EWMH).
    pub wm_check_win: Window,
    pub xlibdisplay: XlibDisplay,
    pub draw: Option<Drw>,
    /// X11 color schemes for borders (different states: normal, tile focus, float focus, snap).
    pub borderscheme: BorderScheme,
    /// X11 color scheme for status bar.
    pub statusscheme: StatusScheme,
    /// X11 cursors for different cursor states.
    pub cursors: [Option<Cursor>; 10],
    /// Last cursor index applied to the X11 root cursor (caching to avoid redundant requests).
    pub last_x11_cursor_index: Option<usize>,
    /// Active non-blocking window animations, keyed by window id.
    pub window_animations: crate::animation::WindowAnimations,
}

impl Default for X11RuntimeConfig {
    fn default() -> Self {
        Self {
            wmatom: WmAtoms::default(),
            netatom: NetAtoms::default(),
            xatom: XAtoms::default(),
            motifatom: 0,
            numlockmask: 0,
            root: 0,
            wm_check_win: 0,
            xlibdisplay: XlibDisplay(std::ptr::null_mut()),
            draw: None,
            borderscheme: BorderScheme::default(),
            statusscheme: StatusScheme::default(),
            cursors: [const { None }; 10],
            last_x11_cursor_index: None,
            window_animations: crate::animation::WindowAnimations::new(),
        }
    }
}

impl X11RuntimeConfig {
    #[inline]
    pub(crate) fn insert_or_replace_window_animation(
        &mut self,
        win: WindowId,
        animation: crate::animation::WindowAnimation,
    ) {
        self.window_animations.insert(win, animation);
    }

    #[inline]
    pub(crate) fn take_window_animation(
        &mut self,
        win: WindowId,
    ) -> Option<crate::animation::WindowAnimation> {
        self.window_animations.remove(&win)
    }
}

pub mod bar;
pub mod client;
pub mod draw;
pub mod events;
pub mod floating;
pub mod grab;
pub mod lifecycle;
pub mod mouse;
pub mod properties;
pub mod randr;

pub use client::update_size_hints_x11;
pub use properties::{
    set_client_state, set_client_tag_prop, update_client_list, update_motif_hints,
    update_window_type, update_wm_hints, window_properties_x11,
};

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

    /// Set the border width of a window.
    /// This is X11-specific as Wayland doesn't support border widths.
    pub fn set_border_width(&self, window: WindowId, width: i32) {
        let x11_win: Window = window.into();
        let _ = self.conn.configure_window(
            x11_win,
            &ConfigureWindowAux::new().border_width(width.max(0) as u32),
        );
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

/// Query the current geometry of an X11 window via `GetGeometry`.
pub fn query_window_rect(x11: &X11BackendRef<'_>, win: WindowId) -> Option<Rect> {
    let reply = x11.conn.get_geometry(win.into()).ok()?.reply().ok()?;
    Some(Rect {
        x: reply.x as i32,
        y: reply.y as i32,
        w: reply.width as i32,
        h: reply.height as i32,
    })
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

    fn raise_window_visual_only(&self, window: WindowId) {
        let x11_win: Window = window.into();
        let _ = self.conn.configure_window(
            x11_win,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
    }

    /// Apply a complete z-order using sibling-based stacking.
    ///
    /// The input is ordered bottom-to-top. The first window is placed at the
    /// bottom via `StackMode::BELOW`, then each subsequent window is stacked
    /// above its predecessor. This produces one `ConfigureWindow` per window
    /// but they are all buffered in the X11 connection and flushed once by the
    /// caller, resulting in a single round-trip.
    fn apply_z_order(&self, windows: &[WindowId]) {
        let mut prev: Option<Window> = None;
        for &window in windows {
            let x11_win: Window = window.into();
            if let Some(sibling) = prev {
                let _ = self.conn.configure_window(
                    x11_win,
                    &ConfigureWindowAux::new()
                        .sibling(sibling)
                        .stack_mode(StackMode::ABOVE),
                );
            } else {
                let _ = self.conn.configure_window(
                    x11_win,
                    &ConfigureWindowAux::new().stack_mode(StackMode::BELOW),
                );
            }
            prev = Some(x11_win);
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

    fn window_exists(&self, window: WindowId) -> bool {
        let x11_win: Window = window.into();
        self.conn.get_window_attributes(x11_win).is_ok()
    }

    fn window_protocol(&self, _window: WindowId) -> crate::backend::WindowProtocol {
        crate::backend::WindowProtocol::X11
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

    fn set_monitor_config(&self, name: &str, config: &crate::config::config_toml::MonitorConfig) {
        let root = self.conn.setup().roots[self.screen_num].root;
        randr::set_monitor_config(self.conn, root, name, config);
    }

    fn get_outputs(&self) -> Vec<crate::backend::BackendOutputInfo> {
        let root = self.conn.setup().roots[self.screen_num].root;
        let outputs = randr::get_outputs(self.conn, root);
        if outputs.is_empty() {
            // Fall back to screen info if no outputs found
            let screen = &self.conn.setup().roots[self.screen_num];
            vec![crate::backend::BackendOutputInfo {
                name: "X11".to_owned(),
                rect: crate::types::Rect {
                    x: 0,
                    y: 0,
                    w: screen.width_in_pixels as i32,
                    h: screen.height_in_pixels as i32,
                },
                scale: 1.0,
                vrr_support: crate::backend::BackendVrrSupport::Unsupported,
                vrr_mode: None,
                vrr_enabled: false,
            }]
        } else {
            outputs
        }
    }
}
