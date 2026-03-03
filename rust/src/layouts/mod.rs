//! Window layout system.
//!
//! This module is the single public face of the layout subsystem.  It is
//! split into four focused sub-modules so that each concern stays small and
//! easy to navigate:
//!
//! | Sub-module    | Responsibility                                              |
//! |---------------|-------------------------------------------------------------|
//! | [`algo`]      | Pure geometry algorithms (tile, monocle, grid, …)          |
//! | [`query`]     | Stateless reads: client counts, layout index resolution     |
//! | [`manager`]   | Stateful operations: arrange, restack, set/cycle layout, …  |
//!
//! ## Layout enum
//!
//! All available layouts are represented by [`LayoutKind`], a simple enum.
//! This enables pattern matching and compile-time exhaustiveness checking.
//!
//! ```text
//! Tile      "+"    master/stack tiling
//! Grid      "#"    square grid
//! Floating  "-"    free floating (no tiling)
//! Monocle  "[M]"   fullscreen stack (monocle)
//! Vert     "|||"   vertical floating (no tiling)
//! Deck     "H[]"   master + stacked deck
//! Overview  "O"    bird's-eye overview of all clients
//! Bstack   "TTT"   bottom-stack (horizontal master row)
//! Horiz    "==="   horizontal floating (no tiling)
//! ```
//!
//! ## Public API surface
//!
//! All symbols that external modules need are re-exported at this level so
//! that callers only ever need `use crate::layouts::…`.

pub mod algo;
pub mod manager;
pub mod query;

use crate::contexts::WmCtx;
use crate::types::Monitor;

/// All available window layouts.
///
/// Each variant corresponds to a specific arrangement algorithm.
/// Properties like `is_tiling()` and `symbol()` are implemented as methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutKind {
    /// Classic master/stack tiling (`+`).
    #[default]
    Tile,
    /// Square grid layout (`#`).
    Grid,
    /// Free floating layout (`-`).
    Floating,
    /// Monocle layout (`[M]`).
    Monocle,
    /// Vertical floating layout (`|||`).
    Vert,
    /// Deck layout (`H[]`).
    Deck,
    /// Overview layout (`O`).
    Overview,
    /// Bottom-stack layout (`TTT`).
    Bstack,
    /// Horizontal floating layout (`===`).
    Horiz,
}

impl LayoutKind {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Tile => "+",
            Self::Grid => "#",
            Self::Floating => "-",
            Self::Monocle => "[M]",
            Self::Vert => "|||",
            Self::Deck => "H[]",
            Self::Overview => "O",
            Self::Bstack => "TTT",
            Self::Horiz => "===",
        }
    }

    pub fn arrange(self, ctx: &mut WmCtx<'_>, m: &mut Monitor) {
        match self {
            Self::Tile => algo::tile(ctx, m),
            Self::Grid => algo::grid(ctx, m),
            Self::Floating => algo::float_left(ctx, m),
            Self::Monocle => algo::monocle(ctx, m),
            Self::Vert => algo::float_left(ctx, m),
            Self::Deck => algo::deck(ctx, m),
            Self::Overview => algo::overviewlayout(ctx, m),
            Self::Bstack => algo::bottom_stack(ctx, m),
            Self::Horiz => algo::float_left(ctx, m),
        }
    }

    pub fn is_tiling(self) -> bool {
        matches!(
            self,
            Self::Tile | Self::Grid | Self::Monocle | Self::Deck | Self::Bstack
        )
    }

    pub fn is_monocle(self) -> bool {
        matches!(self, Self::Monocle)
    }

    pub fn is_overview(self) -> bool {
        matches!(self, Self::Overview)
    }

    pub fn all() -> &'static [LayoutKind] {
        &[
            Self::Tile,
            Self::Grid,
            Self::Floating,
            Self::Monocle,
            Self::Vert,
            Self::Deck,
            Self::Overview,
            Self::Bstack,
            Self::Horiz,
        ]
    }
}

// ── Re-exports: query ─────────────────────────────────────────────────────────
#[allow(unused_imports)]
pub use query::{
    all_client_count, client_count, client_count_mon, find_visible_client, get_current_layout,
    get_current_layout_symbol, selmon_has_tiling_layout,
};

// ── Re-exports: manager ───────────────────────────────────────────────────────
#[allow(unused_imports)]
pub use manager::{
    arrange, arrange_monitor, command_layout, cycle_layout_direction, inc_nmaster_by, restack,
    set_layout, set_mfact, toggle_layout,
};

// ── Re-exports: algorithms (convenience, used by config.rs via `layouts::*`) ──
#[allow(unused_imports)]
pub use algo::{
    bottom_stack, bstackhoriz, deck, dwindle, fibonacci, float_left, gaplessgrid, grid, horizgrid,
    monocle, overviewlayout, spiral, three_column, tile,
};
