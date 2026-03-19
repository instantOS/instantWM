//! Visual appearance: color palette, color schemes, and font configuration.
//!
//! Many items here are public API for user customization and are not all
//! referenced within the crate itself — dead_code is suppressed intentionally.
//!
//! # Color system
//!
//! Each UI element has a "scheme" — a triplet of (foreground, background, detail/accent).
//! Schemes come in two hover variants (`NoHover` / `Hover`).
//!
//! Colors are stored as pre-parsed RGBA values for runtime efficiency.

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------

/// Raw hex color values as RGBA [r, g, b, a].
/// Nothing outside this module should reference these directly — use the
/// typed scheme structs or the `get_*_colors` functions instead.
pub(super) mod palette {
    pub const BG: [f32; 4] = hex_rgba("#121212");
    pub const TEXT: [f32; 4] = hex_rgba("#DFDFDF");
    pub const BLACK: [f32; 4] = hex_rgba("#000000");

    pub const BG_ACCENT: [f32; 4] = hex_rgba("#384252");
    pub const BG_ACCENT_HOVER: [f32; 4] = hex_rgba("#4C5564");
    pub const BG_HOVER: [f32; 4] = hex_rgba("#1C1C1C");

    pub const LIGHT_BLUE: [f32; 4] = hex_rgba("#89B3F7");
    pub const LIGHT_BLUE_HOVER: [f32; 4] = hex_rgba("#a1c2f9");
    pub const BLUE: [f32; 4] = hex_rgba("#536DFE");
    pub const BLUE_HOVER: [f32; 4] = hex_rgba("#758afe");

    pub const LIGHT_GREEN: [f32; 4] = hex_rgba("#81c995");
    pub const LIGHT_GREEN_HOVER: [f32; 4] = hex_rgba("#99d3aa");
    pub const GREEN: [f32; 4] = hex_rgba("#1e8e3e");
    pub const GREEN_HOVER: [f32; 4] = hex_rgba("#4ba465");

    pub const LIGHT_YELLOW: [f32; 4] = hex_rgba("#fdd663");
    pub const LIGHT_YELLOW_HOVER: [f32; 4] = hex_rgba("#fddd82");
    pub const YELLOW: [f32; 4] = hex_rgba("#f9ab00");
    pub const YELLOW_HOVER: [f32; 4] = hex_rgba("#f9bb33");

    pub const LIGHT_RED: [f32; 4] = hex_rgba("#f28b82");
    pub const LIGHT_RED_HOVER: [f32; 4] = hex_rgba("#f4a19a");
    pub const RED: [f32; 4] = hex_rgba("#d93025");
    pub const RED_HOVER: [f32; 4] = hex_rgba("#e05951");

    /// Convert a hex color string to RGBA at compile time.
    /// Expects format "#RRGGBB" or "#RRGGBBAA".
    const fn hex_rgba(hex: &str) -> [f32; 4] {
        let bytes = hex.as_bytes();
        let mut i = 0;
        if bytes[0] == b'#' {
            i = 1;
        }
        let r = hex_digit(bytes[i]) * 16 + hex_digit(bytes[i + 1]);
        let g = hex_digit(bytes[i + 2]) * 16 + hex_digit(bytes[i + 3]);
        let b = hex_digit(bytes[i + 4]) * 16 + hex_digit(bytes[i + 5]);
        let a = if i + 7 < bytes.len() {
            hex_digit(bytes[i + 6]) * 16 + hex_digit(bytes[i + 7])
        } else {
            255
        };
        [
            (r as f32) / 255.0,
            (g as f32) / 255.0,
            (b as f32) / 255.0,
            (a as f32) / 255.0,
        ]
    }

    const fn hex_digit(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => 0,
        }
    }
}

use palette::*;

// ---------------------------------------------------------------------------
// Color table builders
// ---------------------------------------------------------------------------

/// Tag bar color table: `[hover][SchemeTag]`
pub fn get_tag_colors() -> crate::types::TagColorConfigs {
    crate::types::TagColorConfigs {
        no_hover: crate::types::TagColorSet {
            inactive: crate::types::ColorSchemeRgba::new(TEXT, BG, BG),
            filled: crate::types::ColorSchemeRgba::new(TEXT, BG_ACCENT, LIGHT_BLUE),
            focus: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_GREEN, GREEN),
            nofocus: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_YELLOW, YELLOW),
            empty: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_RED, RED),
            urgent: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_RED, RED),
        },
        hover: crate::types::TagColorSet {
            inactive: crate::types::ColorSchemeRgba::new(TEXT, BG_HOVER, BG),
            filled: crate::types::ColorSchemeRgba::new(TEXT, BG_ACCENT_HOVER, LIGHT_BLUE_HOVER),
            focus: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_GREEN_HOVER, GREEN_HOVER),
            nofocus: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_YELLOW_HOVER, YELLOW_HOVER),
            empty: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_RED_HOVER, RED_HOVER),
            urgent: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_RED_HOVER, RED_HOVER),
        },
    }
}

/// Window title color table: `[hover][SchemeWin]`
pub fn get_window_colors() -> crate::types::WindowColorConfigs {
    crate::types::WindowColorConfigs {
        no_hover: crate::types::WindowColorSet {
            focus: crate::types::ColorSchemeRgba::new(TEXT, BG_ACCENT, LIGHT_BLUE),
            normal: crate::types::ColorSchemeRgba::new(TEXT, BG, BG),
            minimized: crate::types::ColorSchemeRgba::new(BG_ACCENT, BG, BG),
            sticky: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_YELLOW, YELLOW),
            sticky_focus: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_GREEN, GREEN),
            overlay: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_YELLOW, YELLOW),
            overlay_focus: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_GREEN, GREEN),
            urgent: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_RED, RED),
        },
        hover: crate::types::WindowColorSet {
            focus: crate::types::ColorSchemeRgba::new(TEXT, BG_ACCENT_HOVER, LIGHT_BLUE_HOVER),
            normal: crate::types::ColorSchemeRgba::new(TEXT, BG_HOVER, BG_HOVER),
            minimized: crate::types::ColorSchemeRgba::new(BG_ACCENT_HOVER, BG, BG),
            sticky: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_YELLOW_HOVER, YELLOW_HOVER),
            sticky_focus: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_GREEN_HOVER, GREEN_HOVER),
            overlay: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_YELLOW_HOVER, YELLOW_HOVER),
            overlay_focus: crate::types::ColorSchemeRgba::new(
                BLACK,
                LIGHT_GREEN_HOVER,
                GREEN_HOVER,
            ),
            urgent: crate::types::ColorSchemeRgba::new(BLACK, LIGHT_RED_HOVER, RED_HOVER),
        },
    }
}

/// Close button color table: `[hover][SchemeClose]`
pub fn get_close_button_colors() -> crate::types::CloseButtonColorConfigs {
    crate::types::CloseButtonColorConfigs {
        no_hover: crate::types::CloseButtonColorSet {
            normal: crate::types::ColorSchemeRgba::new(TEXT, LIGHT_RED, RED),
            locked: crate::types::ColorSchemeRgba::new(TEXT, LIGHT_YELLOW, YELLOW),
            fullscreen: crate::types::ColorSchemeRgba::new(TEXT, LIGHT_RED, RED),
        },
        hover: crate::types::CloseButtonColorSet {
            normal: crate::types::ColorSchemeRgba::new(TEXT, LIGHT_RED_HOVER, RED_HOVER),
            locked: crate::types::ColorSchemeRgba::new(TEXT, LIGHT_YELLOW_HOVER, YELLOW_HOVER),
            fullscreen: crate::types::ColorSchemeRgba::new(TEXT, LIGHT_RED_HOVER, RED_HOVER),
        },
    }
}

/// Border colors.
pub fn get_border_colors() -> crate::types::BorderColorConfig {
    crate::types::BorderColorConfig {
        normal: BG_ACCENT,
        tile_focus: LIGHT_BLUE,
        float_focus: LIGHT_GREEN,
        snap: LIGHT_YELLOW,
    }
}

/// Status bar colors.
pub fn get_status_bar_colors() -> crate::types::StatusColorConfig {
    crate::types::StatusColorConfig {
        fg: TEXT,
        bg: BG,
        detail: BG,
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
