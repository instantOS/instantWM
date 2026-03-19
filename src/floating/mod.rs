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
//! - [`scratchpad`] — named floating windows that can be toggled visible/hidden
//! - [`overlay`] — floating windows anchored to screen edges with animations

mod batch;
mod helpers;
mod movement;
pub mod overlay;
pub mod scratchpad;
mod snap;
mod state;

// ── snap ─────────────────────────────────────────────────────────────────────

/// `SnapDir` is the typed direction enum.
pub use snap::SnapDir;
pub use snap::{change_snap, reset_snap};

// ── movement ─────────────────────────────────────────────────────────────────

/// Keyboard-driven move, resize, centering, and uniform scaling.
pub use movement::{center_window, key_resize, moveresize};

// ── batch ────────────────────────────────────────────────────────────────────

pub use batch::{distribute_clients, restore_all_floating, save_all_floating};

// ── state ────────────────────────────────────────────────────────────────────

pub use state::{
    save_floating_geometry, set_window_mode, toggle_floating, toggle_maximized, WindowMode,
};

// ── overlay ───────────────────────────────────────────────────────────────────

/// Create an overlay window.
pub use overlay::{create_overlay, hide_overlay, set_overlay, set_overlay_mode, show_overlay};

// ── scratchpad ────────────────────────────────────────────────────────────────

/// Make a window a scratchpad.
pub use scratchpad::{scratchpad_make, scratchpad_show_name, scratchpad_toggle, unhide_one};
