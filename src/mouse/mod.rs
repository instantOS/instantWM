//! Mouse-interaction subsystem.
//!
//! This module is split into focused sub-modules:
//!
//! - [`constants`]  — shared numeric constants (sizes, thresholds, keycodes)
//! - [`warp`]       — cursor-warping utilities (`clamp_into`, `warp_to_focus`, …)
//! - [`drag`]       — drag operations aggregator, re-exports from sub-modules:
//!   - [`drag::move_drop`] — move/drop logic, bar hover, edge snap
//!   - [`drag::tag`] — tag bar drag operations
//!   - [`drag::title`] — title bar click/drag
//!   - [`drag::gesture`] — root-window gestures
//! - [`interaction`] — normalized pointer/touch update, end, and cancel events
//! - [`resize`]     — shared corner, aspect, and hover-resize policy
//! - [`slop`]       — slop-based `draw_window`, geometry validation, `apply_window_resize`
//! - [`monitor`]    — monitor-crossing detection after a drag/resize
//!
//! # Typical call flow
//!
//! ```text
//! native pointer/touch event
//!   └─► backend hit-testing / capture
//!         └─► interaction::handle
//!               ├─► active window move/resize
//!               ├─► thresholded title interaction
//!               ├─► tag drag
//!               ├─► sidebar gesture
//!               └─► bottom-bar gesture
//! ```
//!
//! X11's native grab loop and Wayland's event-driven pointer/touch adapters
//! feed the same state machine. They do not implement gesture behavior.

pub mod bindings;
pub mod constants;
pub mod drag;
pub mod hot_corner;
pub mod hover;
pub mod interaction;
pub mod monitor;
pub mod pointer;
pub mod resize;
pub mod slop;
pub mod warp;

// ── Context ─────────────────────────────────────────────────────────────────────

// ── drag ──────────────────────────────────────────────────────────────────────

pub use drag::{
    DragInput, bottom_bar_gesture_finish, drag_tag_finish, drag_tag_motion, sidebar_gesture_begin,
    sidebar_gesture_finish, title_drag_finish, title_drag_motion, update_bottom_bar_gesture,
    update_sidebar_gesture, window_title_mouse_handler,
};

// ── hover ─────────────────────────────────────────────────────────────────────

pub use hot_corner::update_overlay_hot_corner;
pub use hover::{
    SidebarOfferUpdate, clear_hover_offer, commit_x11_hover_offer, set_sidebar_offer,
    update_floating_resize_offer_at, update_selected_resize_offer_at, update_sidebar_offer_at,
};

// ── resize ────────────────────────────────────────────────────────────────────

pub use resize::resize_aspect_mouse;

// ── slop ─────────────────────────────────────────────────────────────────────

pub use slop::draw_window;
