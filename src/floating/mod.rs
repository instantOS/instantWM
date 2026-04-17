//! Floating window management.
//!
//! This module is split into focused sub-modules:
//!
//! - [`snap`]    — snap positions, the navigation matrix, apply/change/reset snap
//! - [`state`]   — save/restore float geometry & border width; set_window_mode;
//!   toggle/set/change floating state; toggle_maximized
//! - [`movement`] — keyboard move, resize, center window, scale client
//! - [`batch`]   — save/restore all floating positions, distribute clients
//! - [`helpers`] — check_floating, visible_client, has_tiling_layout, apply_size
//! - [`scratchpad`] — named floating windows that can be toggled visible/hidden,
//!   with edge-anchored positioning support (overlay scratchpads)

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
pub use movement::{center_window, key_resize};

// ── batch ────────────────────────────────────────────────────────────────────

pub use batch::{distribute_clients, restore_all_floating, save_all_floating};

// ── state ────────────────────────────────────────────────────────────────────

pub use state::{
    WindowMode, save_floating_geometry, set_window_mode, toggle_floating, toggle_maximized,
};

// ── scratchpad ────────────────────────────────────────────────────────────────

pub use scratchpad::{
    OVERLAY_NAME, overlay_create, overlay_toggle, scratchpad_find, scratchpad_hide_name,
    scratchpad_make, scratchpad_show_name, scratchpad_toggle, set_scratchpad_direction, unhide_one,
};
