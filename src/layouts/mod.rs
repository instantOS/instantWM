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
//! | [`manager`]   | Stateful operations: arrange, sync_monitor_z_order, set/cycle layout, …  |
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
//! Deck     "H[]"   master + stacked deck
//! Overview  "O"    bird's-eye overview of all clients
//! Bstack   "TTT"   bottom-stack (horizontal master row)
//! ```
//!
//! ## Public API surface
//!
//! All symbols that external modules need are re-exported at this level so
//! that callers only ever need `use crate::layouts::…`.

pub mod algo;
pub mod manager;
pub mod query;

use std::str::FromStr;

use crate::contexts::WmCtx;
use crate::types::Monitor;

/// All available window layouts.
///
/// Each variant corresponds to a specific arrangement algorithm.
/// Properties like `is_tiling()` and `symbol()` are implemented as methods.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    bincode::Decode,
    bincode::Encode,
    serde::Serialize,
    serde::Deserialize,
    clap::ValueEnum,
)]
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
    /// Deck layout (`H[]`).
    Deck,
    /// Overview layout (`O`).
    Overview,
    /// Bottom-stack layout (`TTT`).
    Bstack,
}

impl LayoutKind {
    pub fn from_name(name: &str) -> Option<Self> {
        Self::from_str(name).ok()
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Tile => "+",
            Self::Grid => "#",
            Self::Floating => "-",
            Self::Monocle => "[M]",
            Self::Deck => "H[]",
            Self::Overview => "O",
            Self::Bstack => "TTT",
        }
    }

    pub fn arrange(self, ctx: &mut WmCtx<'_>, m: &mut Monitor) {
        match self {
            Self::Tile => algo::tile(ctx, m),
            Self::Grid => algo::grid(ctx, m),
            Self::Floating => algo::floating(ctx, m),
            Self::Monocle => algo::monocle(ctx, m),
            Self::Deck => algo::deck(ctx, m),
            Self::Overview => algo::overviewlayout(ctx, m),
            Self::Bstack => algo::bottom_stack(ctx, m),
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
            Self::Deck,
            Self::Overview,
            Self::Bstack,
        ]
    }
}

impl FromStr for LayoutKind {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "tile" | "tiling" => Ok(Self::Tile),
            "grid" => Ok(Self::Grid),
            "float" | "floating" => Ok(Self::Floating),
            "monocle" => Ok(Self::Monocle),
            "deck" => Ok(Self::Deck),
            "overview" => Ok(Self::Overview),
            "bstack" | "bottomstack" => Ok(Self::Bstack),
            _ => Err(()),
        }
    }
}

// ── Re-exports: manager ───────────────────────────────────────────────────────
pub use manager::{
    arrange, cycle_layout_direction, inc_nmaster_by, set_layout, set_mfact, sync_monitor_z_order,
    toggle_layout,
};
