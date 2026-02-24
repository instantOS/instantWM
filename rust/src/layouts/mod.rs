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
//! ## Layout trait implementations
//!
//! Each concrete layout is a zero-sized struct that implements [`Layout`].
//! A `pub static` instance of each is exported so that configuration code
//! can reference them by name (e.g. `&TILE_LAYOUT`).
//!
//! ```text
//! TILE_LAYOUT      "+"    master/stack tiling
//! GRID_LAYOUT      "#"    square grid
//! FLOATING_LAYOUT  "-"    free floating (no tiling)
//! MONOCLE_LAYOUT  "[M]"   fullscreen stack (monocle)
//! VERT_LAYOUT     "|||"   vertical floating (no tiling)
//! DECK_LAYOUT     "H[]"   master + stacked deck
//! OVERVIEW_LAYOUT  "O"    bird's-eye overview of all clients
//! BSTACK_LAYOUT   "TTT"   bottom-stack (horizontal master row)
//! HORIZ_LAYOUT    "==="   horizontal floating (no tiling)
//! ```
//!
//! ## Public API surface
//!
//! All symbols that external modules need are re-exported at this level so
//! that callers only ever need `use crate::layouts::…`.

pub mod algo;
pub mod manager;
pub mod query;

use crate::types::{Layout, Monitor};

// ── Layout implementations ────────────────────────────────────────────────────

/// Classic master/stack tiling layout (`+`).
///
/// The work area is split into a left master column (`mfact * width`) and a
/// right stack column.  See [`algo::tile`] for geometry details.
#[derive(Debug)]
pub struct TileLayout;
pub static TILE_LAYOUT: TileLayout = TileLayout;
impl Layout for TileLayout {
    fn symbol(&self) -> &'static str {
        "+"
    }
    fn arrange(&self, m: &mut Monitor) {
        algo::tile(m);
    }
    fn is_tiling(&self) -> bool {
        true
    }
}

/// Square grid layout (`#`).
///
/// Arranges clients in the smallest square grid that fits them all.
/// Falls back to tile on wide monitors with ≤ 2 clients.
/// See [`algo::grid`] for geometry details.
#[derive(Debug)]
pub struct GridLayout;
pub static GRID_LAYOUT: GridLayout = GridLayout;
impl Layout for GridLayout {
    fn symbol(&self) -> &'static str {
        "#"
    }
    fn arrange(&self, m: &mut Monitor) {
        algo::grid(m);
    }
    fn is_tiling(&self) -> bool {
        true
    }
}

/// Free floating layout (`-`).
///
/// No tiling is performed.  Clients manage their own positions; snap
/// positions are still enforced.  See [`algo::floatl`] for details.
#[derive(Debug)]
pub struct FloatingLayout;
pub static FLOATING_LAYOUT: FloatingLayout = FloatingLayout;
impl Layout for FloatingLayout {
    fn symbol(&self) -> &'static str {
        "-"
    }
    fn arrange(&self, m: &mut Monitor) {
        algo::floatl(m);
    }
    fn is_tiling(&self) -> bool {
        false
    }
}

/// Monocle layout (`[M]`).
///
/// Every tiled client fills the entire work area; only the selected one is
/// raised to the top.  Borders are stripped automatically by [`manager::arrange_monitor`].
/// See [`algo::monocle`] for geometry details.
#[derive(Debug)]
pub struct MonocleLayout;
pub static MONOCLE_LAYOUT: MonocleLayout = MonocleLayout;
impl Layout for MonocleLayout {
    fn symbol(&self) -> &'static str {
        "[M]"
    }
    fn arrange(&self, m: &mut Monitor) {
        algo::monocle(m);
    }
    fn is_tiling(&self) -> bool {
        true
    }
    fn is_monocle(&self) -> bool {
        true
    }
}

/// Vertical floating layout (`|||`).
///
/// Semantically identical to [`FloatingLayout`] — clients are not tiled.
/// The distinct symbol and struct allow configuration code to differentiate
/// between the two floating modes if needed.
#[derive(Debug)]
pub struct VertLayout;
pub static VERT_LAYOUT: VertLayout = VertLayout;
impl Layout for VertLayout {
    fn symbol(&self) -> &'static str {
        "|||"
    }
    fn arrange(&self, m: &mut Monitor) {
        algo::floatl(m);
    }
    fn is_tiling(&self) -> bool {
        false
    }
}

/// Deck layout (`H[]`).
///
/// The master column is split vertically among the first `nmaster` clients.
/// All stack clients are stacked on top of each other in the remaining area —
/// only the topmost is visible (card-deck style).  See [`algo::deck`].
#[derive(Debug)]
pub struct DeckLayout;
pub static DECK_LAYOUT: DeckLayout = DeckLayout;
impl Layout for DeckLayout {
    fn symbol(&self) -> &'static str {
        "H[]"
    }
    fn arrange(&self, m: &mut Monitor) {
        algo::deck(m);
    }
    fn is_tiling(&self) -> bool {
        true
    }
}

/// Overview layout (`O`).
///
/// All clients from every tag are arranged in a square grid so the user can
/// see everything at a glance.  Window cycling and layout cycling skip this
/// layout; it is entered explicitly via a dedicated key binding.
/// See [`algo::overviewlayout`].
#[derive(Debug)]
pub struct OverviewLayout;
pub static OVERVIEW_LAYOUT: OverviewLayout = OverviewLayout;
impl Layout for OverviewLayout {
    fn symbol(&self) -> &'static str {
        "O"
    }
    fn arrange(&self, m: &mut Monitor) {
        algo::overviewlayout(m);
    }
    fn is_tiling(&self) -> bool {
        false
    }
    fn is_overview(&self) -> bool {
        true
    }
}

/// Bottom-stack layout (`TTT`).
///
/// The first `nmaster` clients share a horizontal master row at the top;
/// remaining clients are divided into equal-width vertical columns below.
/// See [`algo::bstack`].
#[derive(Debug)]
pub struct BstackLayout;
pub static BSTACK_LAYOUT: BstackLayout = BstackLayout;
impl Layout for BstackLayout {
    fn symbol(&self) -> &'static str {
        "TTT"
    }
    fn arrange(&self, m: &mut Monitor) {
        algo::bstack(m);
    }
    fn is_tiling(&self) -> bool {
        true
    }
}

/// Horizontal floating layout (`===`).
///
/// Semantically identical to [`FloatingLayout`] — clients are not tiled.
/// See [`algo::floatl`].
#[derive(Debug)]
pub struct HorizLayout;
pub static HORIZ_LAYOUT: HorizLayout = HorizLayout;
impl Layout for HorizLayout {
    fn symbol(&self) -> &'static str {
        "==="
    }
    fn arrange(&self, m: &mut Monitor) {
        algo::floatl(m);
    }
    fn is_tiling(&self) -> bool {
        false
    }
}

// ── Re-exports: query ─────────────────────────────────────────────────────────
//
// These are public API. The compiler warns about "unused" re-exports when no
// other module in the same binary directly names them, but they are consumed
// via `use crate::layouts::*` in config.rs and by external tooling.
#[allow(unused_imports)]
pub use query::{
    all_client_count, client_count, client_count_mon, find_visible_client, get_current_layout,
    get_current_layout_idx, get_current_layout_symbol, is_monocle_layout, is_overview_layout,
    is_tiling_layout, selmon_has_tiling_layout,
};

// ── Re-exports: manager ───────────────────────────────────────────────────────
#[allow(unused_imports)]
pub use manager::{
    arrange, arrange_monitor, command_layout, cycle_layout, cycle_layout_direction, inc_nmaster,
    inc_nmaster_by, restack, set_layout, set_mfact,
};

// ── Re-exports: algorithms (convenience, used by config.rs via `layouts::*`) ──
#[allow(unused_imports)]
pub use algo::{
    bstack, bstackhoriz, deck, dwindle, fibonacci, floatl, gaplessgrid, grid, horizgrid, monocle,
    overviewlayout, spiral, tcl, tile,
};
