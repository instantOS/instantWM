//! Mouse-interaction subsystem.
//!
//! This module is split into focused sub-modules:
//!
//! - [`constants`]  — shared numeric constants (sizes, thresholds, keycodes)
//! - [`warp`]       — cursor-warping utilities (`warp`, `force_warp`, `reset_cursor`, …)
//! - [`grab`]       — X11 pointer-grab helpers (`grab_buttons`, modal grab/ungrab)
//! - [`drag`]       — drag loops: move window, drag tag bar, title-bar click/drag, gestures
//! - [`resize`]     — resize loops: corner resize, aspect resize, hover-resize
//! - [`slop`]       — slop-based `draw_window`, geometry validation, `apply_window_resize`
//! - [`monitor`]    — monitor-crossing detection after a drag/resize
//!
//! # Typical call flow
//!
//! ```text
//! X11 ButtonPress event
//!   └─► events.rs dispatches to one of:
//!         ├─ move_mouse                  (drag module)
//!         ├─ resize_mouse                (resize module)
//!         ├─ hover_resize_mouse          (resize module)
//!         ├─ window_title_mouse_handler  (drag module)
//!         ├─ drag_tag                    (drag module)
//!         └─ gesture_mouse               (drag module)
//! ```
//!
//! All drag/resize functions follow the same skeleton:
//!
//! ```text
//! grab::grab_pointer(cursor_index)
//! loop {
//!     ButtonRelease → break
//!     MotionNotify  → throttle → update geometry
//!     _             → ignore
//! }
//! grab::ungrab(conn)
//! monitor::handle_client_monitor_switch(win)   // if applicable
//! ```

pub mod constants;
pub mod drag;
pub mod grab;
pub mod monitor;
pub mod resize;
pub mod slop;
pub mod warp;

// ── warp ──────────────────────────────────────────────────────────────────────

pub use warp::{force_warp, reset_cursor, warp, warp_cursor_to_client_win, warp_to_focus};

// ── grab ──────────────────────────────────────────────────────────────────────

pub use grab::grab_buttons;

// ── drag ──────────────────────────────────────────────────────────────────────

pub use drag::{
    drag_tag, gesture_mouse, move_mouse, moveresize, window_title_mouse_handler,
    window_title_mouse_handler_right,
};

// ── resize ────────────────────────────────────────────────────────────────────

pub use resize::{
    force_resize_mouse, hover_resize_mouse, is_in_resize_border, resize_aspect_mouse, resize_mouse,
};

// ── slop ─────────────────────────────────────────────────────────────────────

pub use slop::{
    apply_window_resize, apply_window_resize_rect, draw_window, is_valid_window_size,
    is_valid_window_size_rect, parse_slop_output,
};

// ── monitor ───────────────────────────────────────────────────────────────────

pub use monitor::{handle_client_monitor_switch, handle_monitor_switch};

// ── get_cursor_client ─────────────────────────────────────────────────────────

use crate::globals::get_globals;
use crate::types::Client;

/// Return the [`Client`] currently under the mouse pointer, if any.
///
/// Walks every monitor's client list and returns the first client whose
/// geometry contains the current root-pointer position.
///
/// Returns `None` when:
/// * The X11 pointer position cannot be queried.
/// * No client's bounding box contains the pointer.
pub fn get_cursor_client() -> Option<Client> {
    let (ptr_x, ptr_y) = warp::get_root_ptr()?;

    let globals = get_globals();
    for mon in &globals.monitors {
        let mut current = mon.clients;
        while let Some(c_win) = current {
            match globals.clients.get(&c_win) {
                Some(c) => {
                    if c.geo.contains_point(ptr_x, ptr_y) {
                        return Some(c.clone());
                    }
                    current = c.next;
                }
                None => break,
            }
        }
    }

    None
}
