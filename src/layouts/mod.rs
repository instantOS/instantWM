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
//! BottomStack "TTT" bottom-stack (horizontal master row)
//! ```
//!
//! ## Public API surface
//!
//! All symbols that external modules need are re-exported at this level so
//! that callers only ever need `use crate::layouts::…`.

pub mod algo;
pub mod manager;
pub mod query;

use std::collections::HashMap;
use std::str::FromStr;

use crate::geometry::MoveResizeOptions;
use crate::types::client::Client;
use crate::types::{Monitor, Rect, WindowId};

/// The computed geometry output for a single client in a layout pass.
///
/// Returned by the pure `compute` methods on layout algorithms and collected
/// into an [`ArrangePlan`] for batch application.
#[derive(Debug, Clone)]
pub struct LayoutOutput {
    pub win: WindowId,
    pub rect: Rect,
    pub options: MoveResizeOptions,
}

/// Pure-data snapshot of the monitor state changes after an arrange pass.
#[derive(Debug, Clone)]
pub struct MonitorUpdates {
    pub clientcount: u32,
    pub nmaster: i32,
    pub mfact: f32,
    pub work_rect: Rect,
    pub bar_y: i32,
}

/// Complete set of changes required to arrange one monitor.
///
/// Produced by [`Monitor::compute_arrange`] (mutates a snapshot to compute bar geometry)
/// and applied atomically by [`ArrangePlan::apply`].
#[derive(Debug, Clone)]
pub struct ArrangePlan {
    pub monitor_updates: MonitorUpdates,
    pub borders: Vec<(WindowId, i32)>,
    pub client_moves: Vec<LayoutOutput>,
    pub fullscreen_moves: Vec<LayoutOutput>,
    /// Windows whose geometry should be saved before applying moves
    /// (used by overview mode to preserve floating positions).
    pub save_geo: Vec<WindowId>,
    /// True when the plan was computed for overview mode.
    pub is_overview: bool,
}

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
    /// Bottom-stack layout (`TTT`).
    BottomStack,
    /// Horizontal grid layout (`###`).
    HorizGrid,
    /// Gapless grid layout (`g#`).
    GaplessGrid,
    /// Bottom-stack horizontal layout (`===`).
    BStackHoriz,
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
            Self::BottomStack => "TTT",
            Self::HorizGrid => "###",
            Self::GaplessGrid => "g#",
            Self::BStackHoriz => "===",
        }
    }

    /// Compute layout geometry for this layout kind.
    ///
    /// Pure computation — returns `LayoutOutput`s without side effects.
    pub fn compute(
        self,
        monitor: &Monitor,
        clients: &HashMap<WindowId, Client>,
        animated: bool,
    ) -> Vec<LayoutOutput> {
        match self {
            Self::Tile => algo::tile(monitor, clients, animated),
            Self::Grid => algo::grid(monitor, clients, animated),
            Self::Floating => algo::floating(monitor, clients, animated),
            Self::Monocle => algo::monocle(monitor, clients, animated),
            Self::Deck => algo::deck(monitor, clients, animated),
            Self::BottomStack => algo::bottom_stack(monitor, clients, animated),
            Self::HorizGrid => algo::horizgrid(monitor, clients, animated),
            Self::GaplessGrid => algo::gaplessgrid(monitor, clients, animated),
            Self::BStackHoriz => algo::bstackhoriz(monitor, clients, animated),
        }
    }

    pub fn is_tiling(self) -> bool {
        matches!(
            self,
            Self::Tile
                | Self::Grid
                | Self::Monocle
                | Self::Deck
                | Self::BottomStack
                | Self::HorizGrid
                | Self::GaplessGrid
                | Self::BStackHoriz
        )
    }

    pub fn is_monocle(self) -> bool {
        matches!(self, Self::Monocle)
    }

    pub fn all() -> &'static [LayoutKind] {
        &[
            Self::Tile,
            Self::Grid,
            Self::Floating,
            Self::Monocle,
            Self::Deck,
            Self::BottomStack,
            Self::HorizGrid,
            Self::GaplessGrid,
            Self::BStackHoriz,
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
            "bottomstack" => Ok(Self::BottomStack),
            "horizgrid" => Ok(Self::HorizGrid),
            "gaplessgrid" => Ok(Self::GaplessGrid),
            "bstackhoriz" => Ok(Self::BStackHoriz),
            _ => Err(()),
        }
    }
}

// ── Re-exports ────────────────────────────────────────────────────────────────
pub use manager::{
    arrange, cycle_layout_direction, inc_nmaster_by, set_layout, set_mfact, sync_monitor_z_order,
    toggle_layout,
};


