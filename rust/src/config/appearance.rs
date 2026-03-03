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

use crate::types::{ColIndex, SchemeBorder, SchemeClose, SchemeHover, SchemeTag};

// ---------------------------------------------------------------------------
// Color table builders
// ---------------------------------------------------------------------------

/// Tag bar color table: `[hover][SchemeTag][ColIndex]`
/// //TODO should more be imported to make this less awkward?
pub fn get_tag_colors() -> crate::types::TagColorConfigs {
    use palette::*;
    crate::types::TagColorConfigs {
        no_hover: crate::types::TagColorSet {
            inactive: crate::types::ColorSchemeStrings::new(TEXT, BG, BG),
            filled: crate::types::ColorSchemeStrings::new(TEXT, BG_ACCENT, LIGHT_BLUE),
            focus: crate::types::ColorSchemeStrings::new(BLACK, LIGHT_GREEN, GREEN),
            nofocus: crate::types::ColorSchemeStrings::new(BLACK, LIGHT_YELLOW, YELLOW),
            empty: crate::types::ColorSchemeStrings::new(BLACK, LIGHT_RED, RED),
        },
        hover: crate::types::TagColorSet {
            inactive: crate::types::ColorSchemeStrings::new(TEXT, BG_HOVER, BG),
            filled: crate::types::ColorSchemeStrings::new(TEXT, BG_ACCENT_HOVER, LIGHT_BLUE_HOVER),
            focus: crate::types::ColorSchemeStrings::new(BLACK, LIGHT_GREEN_HOVER, GREEN_HOVER),
            nofocus: crate::types::ColorSchemeStrings::new(BLACK, LIGHT_YELLOW_HOVER, YELLOW_HOVER),
            empty: crate::types::ColorSchemeStrings::new(BLACK, LIGHT_RED_HOVER, RED_HOVER),
        },
    }
}

/// Window title color table: `[hover][SchemeWin][ColIndex]`
pub fn get_window_colors() -> crate::types::WindowColorConfigs {
    use palette::*;
    crate::types::WindowColorConfigs {
        no_hover: crate::types::WindowColorSet {
            focus: crate::types::ColorSchemeStrings::new(TEXT, BG_ACCENT, LIGHT_BLUE),
            normal: crate::types::ColorSchemeStrings::new(TEXT, BG, BG),
            minimized: crate::types::ColorSchemeStrings::new(BG_ACCENT, BG, BG),
            sticky: crate::types::ColorSchemeStrings::new(BLACK, LIGHT_YELLOW, YELLOW),
            sticky_focus: crate::types::ColorSchemeStrings::new(BLACK, LIGHT_GREEN, GREEN),
            overlay: crate::types::ColorSchemeStrings::new(BLACK, LIGHT_YELLOW, YELLOW),
            overlay_focus: crate::types::ColorSchemeStrings::new(BLACK, LIGHT_GREEN, GREEN),
        },
        hover: crate::types::WindowColorSet {
            focus: crate::types::ColorSchemeStrings::new(TEXT, BG_ACCENT_HOVER, LIGHT_BLUE_HOVER),
            normal: crate::types::ColorSchemeStrings::new(TEXT, BG_HOVER, BG_HOVER),
            minimized: crate::types::ColorSchemeStrings::new(BG_ACCENT_HOVER, BG, BG),
            sticky: crate::types::ColorSchemeStrings::new(BLACK, LIGHT_YELLOW_HOVER, YELLOW_HOVER),
            sticky_focus: crate::types::ColorSchemeStrings::new(
                BLACK,
                LIGHT_GREEN_HOVER,
                GREEN_HOVER,
            ),
            overlay: crate::types::ColorSchemeStrings::new(BLACK, LIGHT_YELLOW_HOVER, YELLOW_HOVER),
            overlay_focus: crate::types::ColorSchemeStrings::new(
                BLACK,
                LIGHT_GREEN_HOVER,
                GREEN_HOVER,
            ),
        },
    }
}

/// Close button color table: `[hover][SchemeClose][ColIndex]`
pub fn get_close_button_colors() -> crate::types::CloseButtonColorConfigs {
    use palette::*;
    crate::types::CloseButtonColorConfigs {
        no_hover: crate::types::CloseButtonColorSet {
            normal: crate::types::ColorSchemeStrings::new(TEXT, LIGHT_RED, RED),
            locked: crate::types::ColorSchemeStrings::new(TEXT, LIGHT_YELLOW, YELLOW),
            fullscreen: crate::types::ColorSchemeStrings::new(TEXT, LIGHT_RED, RED),
        },
        hover: crate::types::CloseButtonColorSet {
            normal: crate::types::ColorSchemeStrings::new(TEXT, LIGHT_RED_HOVER, RED_HOVER),
            locked: crate::types::ColorSchemeStrings::new(TEXT, LIGHT_YELLOW_HOVER, YELLOW_HOVER),
            fullscreen: crate::types::ColorSchemeStrings::new(TEXT, LIGHT_RED_HOVER, RED_HOVER),
        },
    }
}

/// Border colors.
pub fn get_border_colors() -> crate::types::BorderColorConfig {
    use palette::*;
    crate::types::BorderColorConfig {
        normal: BG_ACCENT.to_string(),
        tile_focus: LIGHT_BLUE.to_string(),
        float_focus: LIGHT_GREEN.to_string(),
        snap: LIGHT_YELLOW.to_string(),
    }
}

/// Status bar colors.
pub fn get_status_bar_colors() -> crate::types::StatusColorConfig {
    use palette::*;
    crate::types::StatusColorConfig {
        fg: TEXT.to_string(),
        bg: BG.to_string(),
        detail: BG.to_string(),
    }
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
pub fn get_fonts() -> Vec<String> {
    vec![
        "Inter-Regular:size=12".to_string(),
        "Fira Code Nerd Font:size=12".to_string(),
    ]
}
