//! X11 backend wrapper.
//!
//! instantWM uses `x11rb::RustConnection` directly throughout the codebase.
//! This wrapper exists to give us a stable place to hang backend-specific
//! functionality while still allowing existing call-sites to use the raw
//! connection.

use x11rb::rust_connection::RustConnection;

pub struct X11Backend {
    pub conn: RustConnection,
    pub screen_num: usize,
}

impl X11Backend {
    pub fn new(conn: RustConnection, screen_num: usize) -> Self {
        Self { conn, screen_num }
    }
}
