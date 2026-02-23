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

/// Re-exported for call-sites in mouse.rs / keyboard.rs that reference these
/// integer constants by name.
pub use snap::{SNAP_LEFT, SNAP_RIGHT, SNAP_TOP};

/// `SnapDir` is the typed direction enum; `change_snap` / `reset_snap` / `apply_snap`
/// / `apply_snap_mut` are used internally and by keyboard.rs.
pub use snap::{apply_snap, apply_snap_mut, change_snap, reset_snap, SnapDir};

// ── state ────────────────────────────────────────────────────────────────────

/// Geometry / border-width persistence, and floating-state transitions.
pub use state::{
    change_floating_win, restore_border_width_win, restore_floating_win, save_bw_win,
    save_floating_win, set_floating, set_tiled, temp_fullscreen, toggle_floating,
};

// ── movement ─────────────────────────────────────────────────────────────────

/// Keyboard-driven move, resize, centering, and uniform scaling.
pub use movement::{
    center_window, downscale_client, key_resize, moveresize, scale_client_win, upscale_client,
};

// ── batch ────────────────────────────────────────────────────────────────────

/// Batch save/restore across all floating clients, and grid distribution.
pub use batch::{distribute_clients, restore_all_floating, save_all_floating};

// ── helpers ──────────────────────────────────────────────────────────────────

/// Stateless query helpers used throughout the module (and occasionally
/// by external callers).
pub use helpers::{apply_size, check_floating, has_tiling_layout, visible_client};
