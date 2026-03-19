//! Layout algorithm implementations.
//!
//! Each sub-module contains one cohesive family of tiling algorithms.
//! All public functions are re-exported here so callers can write
//! `layouts::algo::tile` instead of `layouts::algo::tile::tile`.
//!
//! | Module        | Algorithms                                    |
//! |---------------|-----------------------------------------------|
//! | [`tile`]      | `tile`                                        |
//! | [`monocle`]   | `monocle`                                     |
//! | [`grid`]      | `grid`, `horizgrid`, `gaplessgrid`            |
//! | [`stack`]     | `deck`, `bstack`, `bstackhoriz`               |
//! | [`fibonacci`] | `spiral`, `dwindle`, `fibonacci`              |
//! | [`tcl`]       | `three_column`                                 |
//! | [`overview`]  | `overviewlayout`                              |
//! | [`float`]     | `float_left`, `apply_snap_for_window`, `save_floating` |

mod fibonacci;
mod float;
mod grid;
mod monocle;
mod overview;
mod stack;
mod tcl;
pub(super) mod tile;

// ── tile ─────────────────────────────────────────────────────────────────────
pub use tile::tile;

// ── monocle ──────────────────────────────────────────────────────────────────
pub use monocle::monocle;

// ── grid family ──────────────────────────────────────────────────────────────
pub use grid::{gaplessgrid, grid, horizgrid};

// ── stack family ─────────────────────────────────────────────────────────────
pub use stack::{bottom_stack, bstackhoriz, deck};

// ── fibonacci family ─────────────────────────────────────────────────────────
pub use fibonacci::{dwindle, fibonacci, spiral};

// ── three-column ─────────────────────────────────────────────────────────────
pub use tcl::three_column;

// ── overview ─────────────────────────────────────────────────────────────────
pub use overview::overviewlayout;

// ── floating / snap ──────────────────────────────────────────────────────────
pub use float::{apply_snap_for_window, float_left, save_floating};
