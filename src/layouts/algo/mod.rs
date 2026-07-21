//! Layout algorithm implementations.
//!
//! Each sub-module contains one cohesive family of tiling algorithms.
//! All public functions are re-exported here so callers can write
//! `layouts::algo::tile` instead of `layouts::algo::tile::tile`.
//!
//! | Module        | Algorithms                                    |
//! |---------------|-----------------------------------------------|
//! | [`tile`]      | `tile`                                        |
//! | [`maximized`] | `maximized`                                   |
//! | [`grid`]      | `grid`, `horizgrid`, `gaplessgrid`            |
//! | [`stack`]     | `deck`, `bottom_stack`, `bstackhoriz`         |
//! | [`fibonacci`] | `spiral`, `dwindle`, `fibonacci`              |
//! | [`three_column`] | `three_column`                              |
//! | [`float`]     | `floating` |

mod fibonacci;
mod float;
mod grid;
mod maximized;
mod stack;
mod three_column;
pub(super) mod tile;

// ── tile ─────────────────────────────────────────────────────────────────────
pub use tile::tile;

// ── maximized ────────────────────────────────────────────────────────────────
pub use maximized::maximized;

// ── grid family ──────────────────────────────────────────────────────────────
pub use grid::{gaplessgrid, grid, horizgrid};

// ── stack family ─────────────────────────────────────────────────────────────
pub use stack::{bottom_stack, bstackhoriz, deck};

// ── fibonacci family ─────────────────────────────────────────────────────────
pub use fibonacci::{dwindle, spiral};

// ── three-column ─────────────────────────────────────────────────────────────
pub use three_column::three_column;

// ── floating ─────────────────────────────────────────────────────────────────
pub use float::floating;
