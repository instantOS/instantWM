//! Visual appearance: color palette, color schemes, and font configuration.
//!
//! Many items here are public API for user customization and are not all
//! referenced within the crate itself — dead_code is suppressed intentionally.
#![allow(dead_code)]
//!
//! # Color system
//!
//! Each UI element has a "scheme" — a triplet of (foreground, background, detail/accent).
//! Schemes come in two hover variants (`NoHover` / `Hover`).
//!
//! The raw `&str` hex values live in the private [`palette`] submodule.
//! Everything above that is typed and structured.

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------

/// Raw hex color strings that make up the full palette.
/// Nothing outside this module should reference these directly — use the
/// typed scheme structs or the `get_*_colors` functions instead.
pub(super) mod palette {
    pub const BG: &str = "#121212";
    pub const TEXT: &str = "#DFDFDF";
    pub const BLACK: &str = "#000000";

    pub const BG_ACCENT: &str = "#384252";
    pub const BG_ACCENT_HOVER: &str = "#4C5564";
    pub const BG_HOVER: &str = "#1C1C1C";

    pub const LIGHT_BLUE: &str = "#89B3F7";
    pub const LIGHT_BLUE_HOVER: &str = "#a1c2f9";
    pub const BLUE: &str = "#536DFE";
    pub const BLUE_HOVER: &str = "#758afe";

    pub const LIGHT_GREEN: &str = "#81c995";
    pub const LIGHT_GREEN_HOVER: &str = "#99d3aa";
    pub const GREEN: &str = "#1e8e3e";
    pub const GREEN_HOVER: &str = "#4ba465";

    pub const LIGHT_YELLOW: &str = "#fdd663";
    pub const LIGHT_YELLOW_HOVER: &str = "#fddd82";
    pub const YELLOW: &str = "#f9ab00";
    pub const YELLOW_HOVER: &str = "#f9bb33";

    pub const LIGHT_RED: &str = "#f28b82";
    pub const LIGHT_RED_HOVER: &str = "#f4a19a";
    pub const RED: &str = "#d93025";
    pub const RED_HOVER: &str = "#e05951";
}

// ---------------------------------------------------------------------------
// Scheme enums — used as typed indices into the color tables
// ---------------------------------------------------------------------------

/// Whether the cursor is hovering over the element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeHover {
    NoHover = 0,
    Hover = 1,
}

/// State of a tag button in the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeTag {
    /// No clients on this tag.
    Inactive = 0,
    /// Has clients but not focused on this monitor.
    Filled = 1,
    /// Active tag on the focused monitor.
    Focus = 2,
    /// Active tag on an unfocused monitor.
    NoFocus = 3,
    /// Urgent / special state.
    Empty = 4,
}

/// State of a window title button in the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeWin {
    Focus = 0,
    Normal = 1,
    Minimized = 2,
    Sticky = 3,
    StickyFocus = 4,
    Overlay = 5,
    OverlayFocus = 6,
}

/// State of the close button widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeClose {
    Normal = 0,
    Locked = 1,
    Fullscreen = 2,
}

/// State of the window border.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeBorder {
    Normal = 0,
    TileFocus = 1,
    FloatFocus = 2,
    Snap = 3,
}

/// Which color component to read from a scheme triplet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColIndex {
    Fg = 0,
    Bg = 1,
    Detail = 2,
}

// ---------------------------------------------------------------------------
// Color table builders
// Produce Vec<Vec<Vec<&str>>> in the shape [hover][scheme][col] that the
// rest of the WM consumes.  The inner vec is always [fg, bg, detail].
// ---------------------------------------------------------------------------

/// Tag bar color table: `[hover][SchemeTag][ColIndex]`
pub fn get_tag_colors() -> Vec<Vec<Vec<&'static str>>> {
    use palette::*;
    // Each row is [fg, bg, detail]
    vec![
        // SchemeHover::NoHover
        vec![
            vec![TEXT, BG, BG],                // Inactive
            vec![TEXT, BG_ACCENT, LIGHT_BLUE], // Filled
            vec![BLACK, LIGHT_GREEN, GREEN],   // Focus
            vec![BLACK, LIGHT_YELLOW, YELLOW], // NoFocus
            vec![BLACK, LIGHT_RED, RED],       // Empty
        ],
        // SchemeHover::Hover
        vec![
            vec![TEXT, BG_HOVER, BG],                      // Inactive
            vec![TEXT, BG_ACCENT_HOVER, LIGHT_BLUE_HOVER], // Filled
            vec![BLACK, LIGHT_GREEN_HOVER, GREEN_HOVER],   // Focus
            vec![BLACK, LIGHT_YELLOW_HOVER, YELLOW_HOVER], // NoFocus
            vec![BLACK, LIGHT_RED_HOVER, RED_HOVER],       // Empty
        ],
    ]
}

/// Window title color table: `[hover][SchemeWin][ColIndex]`
pub fn get_window_colors() -> Vec<Vec<Vec<&'static str>>> {
    use palette::*;
    vec![
        // SchemeHover::NoHover
        vec![
            vec![TEXT, BG_ACCENT, LIGHT_BLUE], // Focus
            vec![TEXT, BG, BG],                // Normal
            vec![BG_ACCENT, BG, BG],           // Minimized
            vec![BLACK, LIGHT_YELLOW, YELLOW], // Sticky
            vec![BLACK, LIGHT_GREEN, GREEN],   // StickyFocus
            vec![BLACK, LIGHT_YELLOW, YELLOW], // Overlay
            vec![BLACK, LIGHT_GREEN, GREEN],   // OverlayFocus
        ],
        // SchemeHover::Hover
        vec![
            vec![TEXT, BG_ACCENT_HOVER, LIGHT_BLUE_HOVER], // Focus
            vec![TEXT, BG_HOVER, BG_HOVER],                // Normal
            vec![BG_ACCENT_HOVER, BG, BG],                 // Minimized
            vec![BLACK, LIGHT_YELLOW_HOVER, YELLOW_HOVER], // Sticky
            vec![BLACK, LIGHT_GREEN_HOVER, GREEN_HOVER],   // StickyFocus
            vec![BLACK, LIGHT_YELLOW_HOVER, YELLOW_HOVER], // Overlay
            vec![BLACK, LIGHT_GREEN_HOVER, GREEN_HOVER],   // OverlayFocus
        ],
    ]
}

/// Close button color table: `[hover][SchemeClose][ColIndex]`
pub fn get_close_button_colors() -> Vec<Vec<Vec<&'static str>>> {
    use palette::*;
    vec![
        // SchemeHover::NoHover
        vec![
            vec![TEXT, LIGHT_RED, RED],       // Normal
            vec![TEXT, LIGHT_YELLOW, YELLOW], // Locked
            vec![TEXT, LIGHT_RED, RED],       // Fullscreen
        ],
        // SchemeHover::Hover
        vec![
            vec![TEXT, LIGHT_RED_HOVER, RED_HOVER],       // Normal
            vec![TEXT, LIGHT_YELLOW_HOVER, YELLOW_HOVER], // Locked
            vec![TEXT, LIGHT_RED_HOVER, RED_HOVER],       // Fullscreen
        ],
    ]
}

/// Border colors: `[SchemeBorder as usize]` → single color string.
pub fn get_border_colors() -> Vec<&'static str> {
    use palette::*;
    vec![
        BG_ACCENT,    // Normal
        LIGHT_BLUE,   // TileFocus
        LIGHT_GREEN,  // FloatFocus
        LIGHT_YELLOW, // Snap
    ]
}

/// Status bar colors: `[fg, bg, detail]`
pub fn get_status_bar_colors() -> Vec<&'static str> {
    use palette::*;
    vec![TEXT, BG, BG]
}

// ---------------------------------------------------------------------------
// Typed per-scheme accessors (avoids magic index arithmetic at call sites)
// ---------------------------------------------------------------------------

pub fn tag_color(hover: SchemeHover, scheme: SchemeTag, col: ColIndex) -> &'static str {
    use palette::*;
    match (hover, scheme, col) {
        (SchemeHover::NoHover, SchemeTag::Inactive, ColIndex::Fg) => TEXT,
        (SchemeHover::NoHover, SchemeTag::Inactive, ColIndex::Bg) => BG,
        (SchemeHover::NoHover, SchemeTag::Inactive, ColIndex::Detail) => BG,
        (SchemeHover::NoHover, SchemeTag::Filled, ColIndex::Fg) => TEXT,
        (SchemeHover::NoHover, SchemeTag::Filled, ColIndex::Bg) => BG_ACCENT,
        (SchemeHover::NoHover, SchemeTag::Filled, ColIndex::Detail) => LIGHT_BLUE,
        (SchemeHover::NoHover, SchemeTag::Focus, ColIndex::Fg) => BLACK,
        (SchemeHover::NoHover, SchemeTag::Focus, ColIndex::Bg) => LIGHT_GREEN,
        (SchemeHover::NoHover, SchemeTag::Focus, ColIndex::Detail) => GREEN,
        (SchemeHover::NoHover, SchemeTag::NoFocus, ColIndex::Fg) => BLACK,
        (SchemeHover::NoHover, SchemeTag::NoFocus, ColIndex::Bg) => LIGHT_YELLOW,
        (SchemeHover::NoHover, SchemeTag::NoFocus, ColIndex::Detail) => YELLOW,
        (SchemeHover::NoHover, SchemeTag::Empty, ColIndex::Fg) => BLACK,
        (SchemeHover::NoHover, SchemeTag::Empty, ColIndex::Bg) => LIGHT_RED,
        (SchemeHover::NoHover, SchemeTag::Empty, ColIndex::Detail) => RED,

        (SchemeHover::Hover, SchemeTag::Inactive, ColIndex::Fg) => TEXT,
        (SchemeHover::Hover, SchemeTag::Inactive, ColIndex::Bg) => BG_HOVER,
        (SchemeHover::Hover, SchemeTag::Inactive, ColIndex::Detail) => BG,
        (SchemeHover::Hover, SchemeTag::Filled, ColIndex::Fg) => TEXT,
        (SchemeHover::Hover, SchemeTag::Filled, ColIndex::Bg) => BG_ACCENT_HOVER,
        (SchemeHover::Hover, SchemeTag::Filled, ColIndex::Detail) => LIGHT_BLUE_HOVER,
        (SchemeHover::Hover, SchemeTag::Focus, ColIndex::Fg) => BLACK,
        (SchemeHover::Hover, SchemeTag::Focus, ColIndex::Bg) => LIGHT_GREEN_HOVER,
        (SchemeHover::Hover, SchemeTag::Focus, ColIndex::Detail) => GREEN_HOVER,
        (SchemeHover::Hover, SchemeTag::NoFocus, ColIndex::Fg) => BLACK,
        (SchemeHover::Hover, SchemeTag::NoFocus, ColIndex::Bg) => LIGHT_YELLOW_HOVER,
        (SchemeHover::Hover, SchemeTag::NoFocus, ColIndex::Detail) => YELLOW_HOVER,
        (SchemeHover::Hover, SchemeTag::Empty, ColIndex::Fg) => BLACK,
        (SchemeHover::Hover, SchemeTag::Empty, ColIndex::Bg) => LIGHT_RED_HOVER,
        (SchemeHover::Hover, SchemeTag::Empty, ColIndex::Detail) => RED_HOVER,
    }
}

pub fn window_color(hover: SchemeHover, scheme: SchemeWin, col: ColIndex) -> &'static str {
    use palette::*;
    match (hover, scheme, col) {
        (SchemeHover::NoHover, SchemeWin::Focus, ColIndex::Fg) => TEXT,
        (SchemeHover::NoHover, SchemeWin::Focus, ColIndex::Bg) => BG_ACCENT,
        (SchemeHover::NoHover, SchemeWin::Focus, ColIndex::Detail) => LIGHT_BLUE,
        (SchemeHover::NoHover, SchemeWin::Normal, ColIndex::Fg) => TEXT,
        (SchemeHover::NoHover, SchemeWin::Normal, ColIndex::Bg) => BG,
        (SchemeHover::NoHover, SchemeWin::Normal, ColIndex::Detail) => BG,
        (SchemeHover::NoHover, SchemeWin::Minimized, ColIndex::Fg) => BG_ACCENT,
        (SchemeHover::NoHover, SchemeWin::Minimized, ColIndex::Bg) => BG,
        (SchemeHover::NoHover, SchemeWin::Minimized, ColIndex::Detail) => BG,
        (SchemeHover::NoHover, SchemeWin::Sticky, ColIndex::Fg) => BLACK,
        (SchemeHover::NoHover, SchemeWin::Sticky, ColIndex::Bg) => LIGHT_YELLOW,
        (SchemeHover::NoHover, SchemeWin::Sticky, ColIndex::Detail) => YELLOW,
        (SchemeHover::NoHover, SchemeWin::StickyFocus, ColIndex::Fg) => BLACK,
        (SchemeHover::NoHover, SchemeWin::StickyFocus, ColIndex::Bg) => LIGHT_GREEN,
        (SchemeHover::NoHover, SchemeWin::StickyFocus, ColIndex::Detail) => GREEN,
        (SchemeHover::NoHover, SchemeWin::Overlay, ColIndex::Fg) => BLACK,
        (SchemeHover::NoHover, SchemeWin::Overlay, ColIndex::Bg) => LIGHT_YELLOW,
        (SchemeHover::NoHover, SchemeWin::Overlay, ColIndex::Detail) => YELLOW,
        (SchemeHover::NoHover, SchemeWin::OverlayFocus, ColIndex::Fg) => BLACK,
        (SchemeHover::NoHover, SchemeWin::OverlayFocus, ColIndex::Bg) => LIGHT_GREEN,
        (SchemeHover::NoHover, SchemeWin::OverlayFocus, ColIndex::Detail) => GREEN,

        (SchemeHover::Hover, SchemeWin::Focus, ColIndex::Fg) => TEXT,
        (SchemeHover::Hover, SchemeWin::Focus, ColIndex::Bg) => BG_ACCENT_HOVER,
        (SchemeHover::Hover, SchemeWin::Focus, ColIndex::Detail) => LIGHT_BLUE_HOVER,
        (SchemeHover::Hover, SchemeWin::Normal, ColIndex::Fg) => TEXT,
        (SchemeHover::Hover, SchemeWin::Normal, ColIndex::Bg) => BG_HOVER,
        (SchemeHover::Hover, SchemeWin::Normal, ColIndex::Detail) => BG_HOVER,
        (SchemeHover::Hover, SchemeWin::Minimized, ColIndex::Fg) => BG_ACCENT_HOVER,
        (SchemeHover::Hover, SchemeWin::Minimized, ColIndex::Bg) => BG,
        (SchemeHover::Hover, SchemeWin::Minimized, ColIndex::Detail) => BG,
        (SchemeHover::Hover, SchemeWin::Sticky, ColIndex::Fg) => BLACK,
        (SchemeHover::Hover, SchemeWin::Sticky, ColIndex::Bg) => LIGHT_YELLOW_HOVER,
        (SchemeHover::Hover, SchemeWin::Sticky, ColIndex::Detail) => YELLOW_HOVER,
        (SchemeHover::Hover, SchemeWin::StickyFocus, ColIndex::Fg) => BLACK,
        (SchemeHover::Hover, SchemeWin::StickyFocus, ColIndex::Bg) => LIGHT_GREEN_HOVER,
        (SchemeHover::Hover, SchemeWin::StickyFocus, ColIndex::Detail) => GREEN_HOVER,
        (SchemeHover::Hover, SchemeWin::Overlay, ColIndex::Fg) => BLACK,
        (SchemeHover::Hover, SchemeWin::Overlay, ColIndex::Bg) => LIGHT_YELLOW_HOVER,
        (SchemeHover::Hover, SchemeWin::Overlay, ColIndex::Detail) => YELLOW_HOVER,
        (SchemeHover::Hover, SchemeWin::OverlayFocus, ColIndex::Fg) => BLACK,
        (SchemeHover::Hover, SchemeWin::OverlayFocus, ColIndex::Bg) => LIGHT_GREEN_HOVER,
        (SchemeHover::Hover, SchemeWin::OverlayFocus, ColIndex::Detail) => GREEN_HOVER,
    }
}

pub fn close_button_color(hover: SchemeHover, scheme: SchemeClose, col: ColIndex) -> &'static str {
    use palette::*;
    match (hover, scheme, col) {
        (SchemeHover::NoHover, SchemeClose::Normal, ColIndex::Fg) => TEXT,
        (SchemeHover::NoHover, SchemeClose::Normal, ColIndex::Bg) => LIGHT_RED,
        (SchemeHover::NoHover, SchemeClose::Normal, ColIndex::Detail) => RED,
        (SchemeHover::NoHover, SchemeClose::Locked, ColIndex::Fg) => TEXT,
        (SchemeHover::NoHover, SchemeClose::Locked, ColIndex::Bg) => LIGHT_YELLOW,
        (SchemeHover::NoHover, SchemeClose::Locked, ColIndex::Detail) => YELLOW,
        (SchemeHover::NoHover, SchemeClose::Fullscreen, ColIndex::Fg) => TEXT,
        (SchemeHover::NoHover, SchemeClose::Fullscreen, ColIndex::Bg) => LIGHT_RED,
        (SchemeHover::NoHover, SchemeClose::Fullscreen, ColIndex::Detail) => RED,

        (SchemeHover::Hover, SchemeClose::Normal, ColIndex::Fg) => TEXT,
        (SchemeHover::Hover, SchemeClose::Normal, ColIndex::Bg) => LIGHT_RED_HOVER,
        (SchemeHover::Hover, SchemeClose::Normal, ColIndex::Detail) => RED_HOVER,
        (SchemeHover::Hover, SchemeClose::Locked, ColIndex::Fg) => TEXT,
        (SchemeHover::Hover, SchemeClose::Locked, ColIndex::Bg) => LIGHT_YELLOW_HOVER,
        (SchemeHover::Hover, SchemeClose::Locked, ColIndex::Detail) => YELLOW_HOVER,
        (SchemeHover::Hover, SchemeClose::Fullscreen, ColIndex::Fg) => TEXT,
        (SchemeHover::Hover, SchemeClose::Fullscreen, ColIndex::Bg) => LIGHT_RED_HOVER,
        (SchemeHover::Hover, SchemeClose::Fullscreen, ColIndex::Detail) => RED_HOVER,
    }
}

pub fn border_color(scheme: SchemeBorder) -> &'static str {
    use palette::*;
    match scheme {
        SchemeBorder::Normal => BG_ACCENT,
        SchemeBorder::TileFocus => LIGHT_BLUE,
        SchemeBorder::FloatFocus => LIGHT_GREEN,
        SchemeBorder::Snap => LIGHT_YELLOW,
    }
}

// ---------------------------------------------------------------------------
// Font configuration
// ---------------------------------------------------------------------------

/// Fonts used for bar text rendering (in order of preference / fallback).
pub fn get_fonts() -> Vec<&'static str> {
    vec!["Inter-Regular:size=12", "Fira Code Nerd Font:size=12"]
}
