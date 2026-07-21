//! Persistent manual window-layout system.
//!
//! This module is the single public face of the layout subsystem.  It is
//! split into four focused sub-modules so that each concern stays small and
//! easy to navigate:
//!
//! | Sub-module    | Responsibility                                              |
//! |---------------|-------------------------------------------------------------|
//! | [`tree`]      | Canonical weighted tree and semantic transformations       |
//! | [`algo`]      | Legacy preset geometry kept for compatibility tests         |
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
pub(crate) mod placement;
pub mod query;
pub mod tree;

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
    pub master_count: i32,
    pub master_factor: f32,
    pub bar_height: i32,
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

/// Layout command accepted by configuration and IPC.
///
/// `Floating` changes the persistent mode. Every other variant is a one-shot
/// command which rewrites the manual tree; it is not an active algorithm that
/// will overwrite later edits during arrange.
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
    pub fn name(self) -> &'static str {
        match self {
            Self::Tile => "tile",
            Self::Grid => "grid",
            Self::Floating => "floating",
            Self::Monocle => "monocle",
            Self::Deck => "deck",
            Self::BottomStack => "bottom-stack",
            Self::HorizGrid => "horiz-grid",
            Self::GaplessGrid => "gapless-grid",
            Self::BStackHoriz => "bstack-horiz",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Tile => "Manual Tree",
            Self::Grid => "Grid",
            Self::Floating => "Floating",
            Self::Monocle => "Monocle",
            Self::Deck => "Deck",
            Self::BottomStack => "Bottom Stack",
            Self::HorizGrid => "Horizontal Grid",
            Self::GaplessGrid => "Gapless Grid",
            Self::BStackHoriz => "Bottom Stack Horizontal",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Tile => "Rewrite the manual tree as a master/stack",
            Self::Grid => "Rewrite the manual tree as an even grid",
            Self::Floating => "Windows can be freely moved and resized",
            Self::Monocle => "Rewrite the tree with the focused window dominant",
            Self::Deck => "Rewrite the tree as a non-overlapping master/stack",
            Self::BottomStack => "Rewrite the tree with the master group on top",
            Self::HorizGrid => "Rewrite the tree as a rows-first grid",
            Self::GaplessGrid => "Rewrite the tree as a grid (legacy alias)",
            Self::BStackHoriz => "Rewrite the tree as a horizontal bottom stack",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::from_str(name).ok()
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Tile => "[]",
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
        layout_cfg: &crate::config::config_toml::LayoutConfig,
        animated: bool,
    ) -> Vec<LayoutOutput> {
        match self {
            Self::Tile => algo::tile(monitor, clients, layout_cfg, animated),
            Self::Grid => algo::grid(monitor, clients, layout_cfg, animated),
            Self::Floating => algo::floating(monitor, clients, animated),
            Self::Monocle => algo::monocle(monitor, clients, layout_cfg, animated),
            Self::Deck => algo::deck(monitor, clients, layout_cfg, animated),
            Self::BottomStack => algo::bottom_stack(monitor, clients, layout_cfg, animated),
            Self::HorizGrid => algo::horizgrid(monitor, clients, layout_cfg, animated),
            Self::GaplessGrid => algo::gaplessgrid(monitor, clients, layout_cfg, animated),
            Self::BStackHoriz => algo::bstackhoriz(monitor, clients, layout_cfg, animated),
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
            "bottomstack" | "bottom-stack" | "bstack" => Ok(Self::BottomStack),
            "horizgrid" => Ok(Self::HorizGrid),
            "gaplessgrid" => Ok(Self::GaplessGrid),
            "bstackhoriz" => Ok(Self::BStackHoriz),
            _ => Err(()),
        }
    }
}

// ── Re-exports ────────────────────────────────────────────────────────────────
pub use manager::{
    apply_tree_preset, arrange, begin_keyboard_tree_placement, center_keyboard_tree_placement,
    cycle_keyboard_tree_placement, cycle_layout_direction, finish_keyboard_tree_placement,
    focus_tree_neighbor, inc_master_count_by, place_tree_at_point, preview_tree_at_point,
    promote_tree, resize_keyboard_tree_placement, resize_tree, resize_tree_smart, set_layout,
    set_master_factor, step_keyboard_tree_placement, swap_keyboard_tree_placement,
    swap_tree_neighbor, sync_monitor_z_order, toggle_layout,
};
