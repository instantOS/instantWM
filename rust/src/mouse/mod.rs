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
//!         ├─ hover_resize_mouse          (hover module)
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
mod cursor;
pub mod drag;
pub mod grab;
pub mod hover;
pub mod monitor;
pub mod resize;
pub mod slop;
pub mod warp;

// ── Context ─────────────────────────────────────────────────────────────────────

// ── warp ──────────────────────────────────────────────────────────────────────

pub use warp::reset_cursor;

// ── grab ──────────────────────────────────────────────────────────────────────

// ── drag ──────────────────────────────────────────────────────────────────────

pub use drag::{
    drag_tag, drag_tag_begin, drag_tag_finish, drag_tag_motion, gesture_mouse, move_mouse,
    title_drag_finish, title_drag_motion, window_title_mouse_handler,
    window_title_mouse_handler_right,
};

// moveresize lives in floating::movement; re-exported here so keybindings.rs
// can use the single import path `crate::mouse::moveresize`.
pub use crate::floating::moveresize;

// ── hover ─────────────────────────────────────────────────────────────────────

pub use hover::{
    floating_to_tiled_hover, get_cursor_client_win, handle_floating_resize_hover,
    handle_sidebar_hover, hover_resize_mouse,
};

// ── resize ────────────────────────────────────────────────────────────────────

pub use resize::{resize_aspect_mouse, resize_mouse_directional, resize_mouse_from_cursor};

// ── slop ─────────────────────────────────────────────────────────────────────

pub use slop::draw_window;
