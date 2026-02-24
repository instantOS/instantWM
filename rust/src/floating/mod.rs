//! Floating window management.
//!
//! This module is split into focused sub-modules:
//!
//! - [`snap`]      — snap positions, the navigation matrix, apply/change/reset snap
//! - [`state`]     — save/restore float geometry & border width; apply_float_change;
//!                   toggle/set/change floating state; temp_fullscreen
//! - [`movement`]  — keyboard move, resize, center window, scale client
//! - [`batch`]     — save/restore all floating positions, distribute clients
//! - [`helpers`]   — check_floating, visible_client, has_tiling_layout, apply_size

mod batch;
mod helpers;
mod movement;
mod snap;
mod state;

// ── snap ─────────────────────────────────────────────────────────────────────

/// `SnapDir` is the typed direction enum.
pub use snap::SnapDir;
pub use snap::{apply_snap, change_snap, reset_snap};

// ── movement ─────────────────────────────────────────────────────────────────

/// Keyboard-driven move, resize, centering, and uniform scaling.
pub use movement::{
    center_window, downscale_client, key_resize, moveresize, scale_client_win, upscale_client,
};

// ── batch ────────────────────────────────────────────────────────────────────

pub use batch::{distribute_clients, restore_all_floating, save_all_floating};

// ── helpers ──────────────────────────────────────────────────────────────────

pub use helpers::{apply_size, check_floating, has_tiling_layout, visible_client};

// ── state ────────────────────────────────────────────────────────────────────

pub use state::{
    apply_float_change, change_floating_win, restore_floating_win, save_floating_win, set_floating,
    set_floating_in_place, set_tiled, temp_fullscreen, toggle_floating,
};
