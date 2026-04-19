//! Mouse-interaction subsystem.
//!
//! This module is split into focused sub-modules:
//!
//! - [`constants`]  — shared numeric constants (sizes, thresholds, keycodes)
//! - [`warp`]       — cursor-warping utilities (`warp_into`, `warp_to_focus`, `reset_cursor`, …)
//! - [`drag`]       — drag operations aggregator, re-exports from sub-modules:
//!   - [`drag::move_drop`] — move/drop logic, bar hover, edge snap
//!   - [`drag::tag`] — tag bar drag operations
//!   - [`drag::title`] — title bar click/drag
//!   - [`drag::gesture`] — root-window gestures
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
//!         ├─ run_x11_hover_resize_offer_loop (hover module)
//!         ├─ window_title_mouse_handler  (drag module)
//!         ├─ drag_tag                    (drag module)
//!         └─ sidebar_gesture_begin       (drag module)
//! ```
//!
//! All drag/resize functions follow the same skeleton:
//!
//! ```text
//! backend::x11::grab::grab_pointer(cursor_index)
//! loop {
//!     ButtonRelease → break
//!     MotionNotify  → throttle → update geometry
//!     _             → ignore
//! }
//! backend::x11::grab::ungrab(ctx)
//! monitor::handle_client_monitor_switch(win)   // if applicable
//! ```

pub mod constants;
mod cursor;
pub mod drag;
pub mod hover;
pub mod monitor;
pub mod pointer;
pub mod resize;
pub mod slop;
pub mod warp;

// ── Context ─────────────────────────────────────────────────────────────────────

// ── warp ──────────────────────────────────────────────────────────────────────

pub use cursor::set_cursor_style;
pub use warp::reset_cursor;

// ── drag ──────────────────────────────────────────────────────────────────────

pub use drag::{
    begin_keyboard_move, drag_tag, drag_tag_finish, drag_tag_motion, finish_sidebar_gesture,
    sidebar_gesture_begin, title_drag_finish, title_drag_motion, update_sidebar_gesture,
    window_title_mouse_handler,
};

// ── hover ─────────────────────────────────────────────────────────────────────

pub use hover::{
    clear_hover_offer, commit_x11_hover_offer, handle_x11_floating_to_tiled_hover_offer,
    run_x11_hover_resize_offer_loop, update_floating_resize_offer_at,
    update_selected_resize_offer_at, update_sidebar_offer_at,
};

// ── resize ────────────────────────────────────────────────────────────────────

pub use resize::{resize_aspect_mouse, resize_mouse_from_cursor};

// ── slop ─────────────────────────────────────────────────────────────────────

pub use slop::draw_window;
