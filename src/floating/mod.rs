//! Floating window management.
//!
//! This module is split into focused sub-modules:
//!
//! - [`snap`]    — snap positions, the navigation matrix, apply/change/reset snap
//! - [`state`]   — save/restore float geometry & border width; set_window_mode;
//!   toggle/set/change floating state; internal client-maximize transitions
//! - [`movement`] — keyboard move, resize, center window, scale client
//! - [`batch`]   — distribute floating clients
//! - [`helpers`] — visible_client, has_tiling_layout, apply_size
//! - [`scratchpad`] — named floating windows that can be toggled visible/hidden,
//!   with optional edge-anchored positioning

mod batch;
mod helpers;
mod movement;
pub mod scratchpad;
mod snap;
mod state;

// ── snap ─────────────────────────────────────────────────────────────────────

pub use snap::{change_snap, reset_snap};

// ── movement ─────────────────────────────────────────────────────────────────

/// Keyboard-driven move, resize, centering, and uniform scaling.
pub use movement::{center_window, key_move, key_resize};

// ── batch ────────────────────────────────────────────────────────────────────

pub use batch::distribute_clients;

// ── state ────────────────────────────────────────────────────────────────────

pub(crate) use state::toggle_client_maximized;
pub use state::{WindowModeChange, WindowModeRequest, set_window_mode, toggle_floating};

// ── scratchpad ────────────────────────────────────────────────────────────────

pub use scratchpad::{
    DEFAULT_EDGE_SCRATCHPAD_NAME, edge_scratchpad_create, scratchpad_hide_name, scratchpad_make,
    scratchpad_show_name, scratchpad_toggle, set_scratchpad_direction,
};
