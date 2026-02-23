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
//! | [`tcl`]       | `tcl`                                         |
//! | [`overview`]  | `overviewlayout`                              |
//! | [`float`]     | `floatl`, `apply_snap_for_window`, `save_floating` |

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
pub use stack::{bstack, bstackhoriz, deck};

// ── fibonacci family ─────────────────────────────────────────────────────────
pub use fibonacci::{dwindle, fibonacci, spiral};

// ── three-column ─────────────────────────────────────────────────────────────
pub use tcl::tcl;

// ── overview ─────────────────────────────────────────────────────────────────
pub use overview::overviewlayout;

// ── floating / snap ──────────────────────────────────────────────────────────
pub use float::{floatl, save_floating};

// `apply_snap_for_window` is used internally by floatl; it is also exported
// for any call-site that needs to apply snap geometry directly.
#[allow(unused_imports)]
pub use float::apply_snap_for_window;
