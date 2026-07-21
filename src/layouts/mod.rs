//! Persistent manual window-layout system.
//!
//! This module is the single public face of the layout subsystem.  It is
//! split into focused sub-modules so that each concern stays small and
//! easy to navigate:
//!
//! | Sub-module | Responsibility |
//! |------------|----------------|
//! | [`tree`] | Canonical weighted tree and private transformation algorithms |
//! | [`algo`] | Non-tree floating and maximized presentations |
//! | [`query`] | Stateless client-count and animation queries |
//! | `placement` | Gap, border, and preview geometry |
//! | `keyboard_placement` | Keyboard placement-session orchestration |
//! | [`manager`] | Arrange application, z-order, pointer interaction, and layout commands |
//!
//! ## Layout commands and presentation state
//!
//! [`PresentationMode`] is persistent per tag. [`LayoutCommand`] is an
//! imperative request: it either changes that presentation or rewrites the
//! manual tree using a preset. Keeping these types separate prevents a
//! one-shot preset such as grid from becoming bogus persistent state.
//!
//! ```text
//! Tile      "+"    master/stack tiling
//! Grid      "#"    square grid
//! Floating  "-"    free floating (no tiling)
//! Maximized "[M]"  focused full-work-area tiled stack
//! BottomStack "TTT" bottom-stack (horizontal master row)
//! ```
//!
//! ## Public API surface
//!
//! All symbols that external modules need are re-exported at this level so
//! that callers only ever need `use crate::layouts::…`.

pub mod algo;
mod keyboard_placement;
pub mod manager;
pub(crate) mod placement;
pub mod query;
pub mod tree;

use std::str::FromStr;

use crate::geometry::MoveResizeOptions;
use crate::types::{Rect, WindowId};

/// Persistent way in which a tag presents its authoritative manual tree.
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
)]
pub enum PresentationMode {
    #[default]
    Tiled,
    Floating,
    Maximized,
}

impl PresentationMode {
    pub const fn is_tiling(self) -> bool {
        !matches!(self, Self::Floating)
    }

    pub const fn is_maximized(self) -> bool {
        matches!(self, Self::Maximized)
    }

    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Tiled => "[]",
            Self::Floating => "-",
            Self::Maximized => "[M]",
        }
    }
}

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

/// Complete set of changes required to arrange one monitor.
///
/// Produced by [`crate::types::Monitor::compute_arrange`] (mutates a snapshot to compute bar geometry)
/// and applied atomically by [`ArrangePlan::apply`].
#[derive(Debug, Clone)]
pub struct ArrangePlan {
    pub bar_height: i32,
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
/// `Floating` and `Maximized` are persistent presentation modes. Every other
/// variant is a one-shot command which rewrites the manual tree; it is not an
/// active algorithm that will overwrite later edits during arrange.
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
pub enum LayoutCommand {
    /// Classic master/stack tiling (`+`).
    #[default]
    Tile,
    /// Square grid layout (`#`).
    Grid,
    /// Free floating layout (`-`).
    Floating,
    /// Maximized tiled stack (`[M]`). The underlying manual tree is preserved.
    Maximized,
    /// Bottom-stack layout (`TTT`).
    BottomStack,
    /// Horizontal grid layout (`###`).
    HorizGrid,
    /// Bottom-stack horizontal layout (`===`).
    BStackHoriz,
}

impl LayoutCommand {
    pub const fn presentation(self) -> PresentationMode {
        match self {
            Self::Floating => PresentationMode::Floating,
            Self::Maximized => PresentationMode::Maximized,
            Self::Tile | Self::Grid | Self::BottomStack | Self::HorizGrid | Self::BStackHoriz => {
                PresentationMode::Tiled
            }
        }
    }

    pub const fn tree_preset(self) -> Option<tree::Preset> {
        match self {
            Self::Tile => Some(tree::Preset::MasterStack),
            Self::Grid => Some(tree::Preset::Grid),
            Self::HorizGrid => Some(tree::Preset::HorizontalGrid),
            Self::BottomStack => Some(tree::Preset::BottomStack),
            Self::BStackHoriz => Some(tree::Preset::BottomStackHorizontal),
            Self::Floating | Self::Maximized => None,
        }
    }

    pub const fn from_tree_preset(preset: tree::Preset) -> Option<Self> {
        match preset {
            tree::Preset::MasterStack => Some(Self::Tile),
            tree::Preset::Grid => Some(Self::Grid),
            tree::Preset::HorizontalGrid => Some(Self::HorizGrid),
            tree::Preset::BottomStack => Some(Self::BottomStack),
            tree::Preset::BottomStackHorizontal => Some(Self::BStackHoriz),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Tile => "tile",
            Self::Grid => "grid",
            Self::Floating => "floating",
            Self::Maximized => "maximized",
            Self::BottomStack => "bottom-stack",
            Self::HorizGrid => "horiz-grid",
            Self::BStackHoriz => "bstack-horiz",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Tile => "Manual Tree",
            Self::Grid => "Grid",
            Self::Floating => "Floating",
            Self::Maximized => "Maximized",
            Self::BottomStack => "Bottom Stack",
            Self::HorizGrid => "Horizontal Grid",
            Self::BStackHoriz => "Bottom Stack Horizontal",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Tile => "Rewrite the manual tree as a master/stack",
            Self::Grid => "Rewrite the manual tree as an even grid",
            Self::Floating => "Windows can be freely moved and resized",
            Self::Maximized => "Stack tiled windows at full work-area size",
            Self::BottomStack => "Rewrite the tree with the master group on top",
            Self::HorizGrid => "Rewrite the tree as a rows-first grid",
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
            Self::Maximized => "[M]",
            Self::BottomStack => "TTT",
            Self::HorizGrid => "###",
            Self::BStackHoriz => "===",
        }
    }

    /// Whether executing this command leaves the tag in a tiled presentation.
    pub const fn results_in_tiling(self) -> bool {
        self.presentation().is_tiling()
    }

    /// Canonical commands shown by the CLI and visited by layout cycling.
    ///
    pub fn all() -> &'static [LayoutCommand] {
        &[
            Self::Tile,
            Self::Grid,
            Self::Floating,
            Self::Maximized,
            Self::BottomStack,
            Self::HorizGrid,
            Self::BStackHoriz,
        ]
    }
}

impl FromStr for LayoutCommand {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "tile" => Ok(Self::Tile),
            "grid" => Ok(Self::Grid),
            "floating" => Ok(Self::Floating),
            "maximized" => Ok(Self::Maximized),
            "bottom-stack" => Ok(Self::BottomStack),
            "horiz-grid" => Ok(Self::HorizGrid),
            "bstack-horiz" => Ok(Self::BStackHoriz),
            _ => Err(()),
        }
    }
}

// ── Re-exports ────────────────────────────────────────────────────────────────
pub use keyboard_placement::{
    TreePlacementStart, begin_tree_placement, center_keyboard_tree_placement,
    cycle_keyboard_tree_placement, finish_keyboard_tree_placement, resize_keyboard_tree_placement,
    step_keyboard_tree_placement, swap_keyboard_tree_placement,
};
pub use manager::{
    apply_tree_preset, arrange, cycle_layout_direction, focus_tree_neighbor, inc_master_count_by,
    place_tree_at_point, preview_tree_at_point, promote_tree, resize_tree, resize_tree_smart,
    set_layout, swap_tree_neighbor, sync_monitor_z_order, toggle_tiling_maximized,
};

#[cfg(test)]
mod command_tests {
    use super::{LayoutCommand, PresentationMode};
    use crate::layouts::tree::Preset;

    #[test]
    fn presentation_commands_cannot_be_mistaken_for_tree_presets() {
        assert_eq!(LayoutCommand::Floating.tree_preset(), None);
        assert_eq!(LayoutCommand::Maximized.tree_preset(), None);
        assert_eq!(
            LayoutCommand::Floating.presentation(),
            PresentationMode::Floating
        );
        assert_eq!(
            LayoutCommand::Maximized.presentation(),
            PresentationMode::Maximized
        );
    }

    #[test]
    fn structural_commands_always_select_tiled_presentation() {
        for command in LayoutCommand::all()
            .iter()
            .copied()
            .filter(|command| command.tree_preset().is_some())
        {
            assert_eq!(command.presentation(), PresentationMode::Tiled);
            assert!(command.results_in_tiling());
        }
    }

    #[test]
    fn canonical_presets_map_back_to_cycle_commands() {
        for preset in [
            Preset::MasterStack,
            Preset::Grid,
            Preset::HorizontalGrid,
            Preset::BottomStack,
            Preset::BottomStackHorizontal,
        ] {
            let command = LayoutCommand::from_tree_preset(preset).unwrap();
            assert_eq!(command.tree_preset(), Some(preset));
        }
    }

    #[test]
    fn canonical_command_list_has_no_duplicate_tree_presets() {
        let presets = LayoutCommand::all()
            .iter()
            .filter_map(|command| command.tree_preset())
            .collect::<std::collections::HashSet<_>>();
        let structural_count = LayoutCommand::all()
            .iter()
            .filter(|command| command.tree_preset().is_some())
            .count();
        assert_eq!(presets.len(), structural_count);
    }
}
