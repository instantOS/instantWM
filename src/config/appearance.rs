//! Built-in colour themes and font configuration.
//!
//! Backends consume the resolved, typed colour tables. Theme selection and
//! user overrides are handled while loading TOML, before runtime state exists.

use crate::config::config_toml::{ColorConfig, ColorTheme};
use crate::types::{
    BorderColorConfig, CloseButtonColorConfigs, CloseButtonColorSet, ColorSchemeRgba,
    StatusColorConfig, TagColorConfigs, TagColorSet, WindowColorConfigs, WindowColorSet,
};

type Rgba = [f32; 4];

#[derive(Clone, Copy)]
struct Accent {
    fill: Rgba,
    detail: Rgba,
    hover_fill: Rgba,
    hover_detail: Rgba,
}

impl Accent {
    const fn solid(fill: &str, hover_fill: &str) -> Self {
        Self {
            fill: hex(fill),
            detail: hex(fill),
            hover_fill: hex(hover_fill),
            hover_detail: hex(hover_fill),
        }
    }

    const fn layered(fill: &str, detail: &str, hover_fill: &str, hover_detail: &str) -> Self {
        Self {
            fill: hex(fill),
            detail: hex(detail),
            hover_fill: hex(hover_fill),
            hover_detail: hex(hover_detail),
        }
    }
}

/// Semantic colours consumed by instantWM's UI states.
///
/// Theme authors choose colours by purpose, rather than matching a particular
/// theme's hue names. For example, `focused` may be green in one theme and
/// purple in another without making the field name misleading.
#[derive(Clone, Copy)]
struct ThemePalette {
    background: Rgba,
    foreground: Rgba,
    foreground_on_accent: Rgba,
    background_hover: Rgba,
    surface: Rgba,
    surface_hover: Rgba,
    /// Filled tags, focused title surface, and focused tiled border.
    primary: Accent,
    /// Active/focused tags, sticky focus, and focused floating border.
    focused: Accent,
    /// Sticky/unfocused-active states and snap indicators.
    special: Accent,
    /// Urgent states and close buttons.
    urgent: Accent,
}

const fn hex(hex: &str) -> Rgba {
    const fn digit(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => 0,
        }
    }
    let b = hex.as_bytes();
    let i = if b[0] == b'#' { 1 } else { 0 };
    [
        (digit(b[i]) * 16 + digit(b[i + 1])) as f32 / 255.0,
        (digit(b[i + 2]) * 16 + digit(b[i + 3])) as f32 / 255.0,
        (digit(b[i + 4]) * 16 + digit(b[i + 5])) as f32 / 255.0,
        1.0,
    ]
}

fn palette(theme: ColorTheme) -> ThemePalette {
    match theme {
        ColorTheme::Instantos => ThemePalette {
            background: hex("121212"),
            foreground: hex("dfdfdf"),
            foreground_on_accent: hex("000000"),
            background_hover: hex("1c1c1c"),
            surface: hex("384252"),
            surface_hover: hex("4c5564"),
            primary: Accent::layered("89b3f7", "536dfe", "a1c2f9", "758afe"),
            focused: Accent::layered("81c995", "1e8e3e", "99d3aa", "4ba465"),
            special: Accent::layered("fdd663", "f9ab00", "fddd82", "f9bb33"),
            urgent: Accent::layered("f28b82", "d93025", "f4a19a", "e05951"),
        },
        ColorTheme::CatppuccinLatte => ThemePalette {
            background: hex("eff1f5"),
            foreground: hex("4c4f69"),
            foreground_on_accent: hex("dce0e8"),
            background_hover: hex("e6e9ef"),
            surface: hex("ccd0da"),
            surface_hover: hex("bcc0cc"),
            primary: Accent::solid("1e66f5", "7287fd"),
            focused: Accent::solid("40a02b", "70b433"),
            special: Accent::solid("df8e1d", "fead3d"),
            urgent: Accent::solid("d20f39", "e64553"),
        },
        ColorTheme::CatppuccinFrappe => ThemePalette {
            background: hex("303446"),
            foreground: hex("c6d0f5"),
            foreground_on_accent: hex("232634"),
            background_hover: hex("292c3c"),
            surface: hex("414559"),
            surface_hover: hex("51576d"),
            primary: Accent::solid("8caaee", "babbf1"),
            focused: Accent::solid("a6d189", "b5d49a"),
            special: Accent::solid("e5c890", "efcf8e"),
            urgent: Accent::solid("e78284", "ea999c"),
        },
        ColorTheme::CatppuccinMacchiato => ThemePalette {
            background: hex("24273a"),
            foreground: hex("cad3f5"),
            foreground_on_accent: hex("181926"),
            background_hover: hex("1e2030"),
            surface: hex("363a4f"),
            surface_hover: hex("494d64"),
            primary: Accent::solid("8aadf4", "b7bdf8"),
            focused: Accent::solid("a6da95", "b8df9f"),
            special: Accent::solid("eed49f", "f5dcaa"),
            urgent: Accent::solid("ed8796", "f49da6"),
        },
        ColorTheme::CatppuccinMocha => ThemePalette {
            background: hex("1e1e2e"),
            foreground: hex("cdd6f4"),
            foreground_on_accent: hex("11111b"),
            background_hover: hex("181825"),
            surface: hex("313244"),
            surface_hover: hex("45475a"),
            primary: Accent::solid("89b4fa", "b4befe"),
            focused: Accent::solid("a6e3a1", "b9e6b5"),
            special: Accent::solid("f9e2af", "f5e0b5"),
            urgent: Accent::solid("f38ba8", "f5a2b8"),
        },
        ColorTheme::Nord => ThemePalette {
            background: hex("2e3440"),
            foreground: hex("eceff4"),
            foreground_on_accent: hex("242933"),
            background_hover: hex("343b49"),
            surface: hex("3b4252"),
            surface_hover: hex("4c566a"),
            primary: Accent::solid("81a1c1", "88c0d0"),
            focused: Accent::solid("a3be8c", "b1c89d"),
            special: Accent::solid("ebcb8b", "efd49f"),
            urgent: Accent::solid("bf616a", "cf7b83"),
        },
        ColorTheme::Gruvbox => ThemePalette {
            background: hex("282828"),
            foreground: hex("ebdbb2"),
            foreground_on_accent: hex("1d2021"),
            background_hover: hex("32302f"),
            surface: hex("3c3836"),
            surface_hover: hex("504945"),
            primary: Accent::solid("83a598", "8ec07c"),
            focused: Accent::solid("b8bb26", "98971a"),
            special: Accent::solid("fabd2f", "d79921"),
            urgent: Accent::solid("fb4934", "cc241d"),
        },
    }
}

/// Return every resolved colour table for a built-in theme.
pub fn colors(theme: ColorTheme) -> ColorConfig {
    let p = palette(theme);
    let scheme = ColorSchemeRgba::new;
    ColorConfig {
        tag: TagColorConfigs {
            no_hover: TagColorSet {
                inactive: scheme(p.foreground, p.background, p.background),
                filled: scheme(p.foreground, p.surface, p.primary.detail),
                focus: scheme(p.foreground_on_accent, p.focused.fill, p.focused.detail),
                nofocus: scheme(p.foreground_on_accent, p.special.fill, p.special.detail),
                empty: scheme(p.foreground_on_accent, p.urgent.fill, p.urgent.detail),
                urgent: scheme(p.foreground_on_accent, p.urgent.fill, p.urgent.detail),
            },
            hover: TagColorSet {
                inactive: scheme(p.foreground, p.background_hover, p.background),
                filled: scheme(p.foreground, p.surface_hover, p.primary.hover_detail),
                focus: scheme(
                    p.foreground_on_accent,
                    p.focused.hover_fill,
                    p.focused.hover_detail,
                ),
                nofocus: scheme(
                    p.foreground_on_accent,
                    p.special.hover_fill,
                    p.special.hover_detail,
                ),
                empty: scheme(
                    p.foreground_on_accent,
                    p.urgent.hover_fill,
                    p.urgent.hover_detail,
                ),
                urgent: scheme(
                    p.foreground_on_accent,
                    p.urgent.hover_fill,
                    p.urgent.hover_detail,
                ),
            },
        },
        window: WindowColorConfigs {
            no_hover: WindowColorSet {
                focus: scheme(p.foreground, p.surface, p.primary.detail),
                normal: scheme(p.foreground, p.background, p.background),
                minimized: scheme(p.surface, p.background, p.background),
                sticky: scheme(p.foreground_on_accent, p.special.fill, p.special.detail),
                sticky_focus: scheme(p.foreground_on_accent, p.focused.fill, p.focused.detail),
                edge_scratchpad: scheme(p.foreground_on_accent, p.special.fill, p.special.detail),
                edge_scratchpad_focus: scheme(
                    p.foreground_on_accent,
                    p.focused.fill,
                    p.focused.detail,
                ),
                urgent: scheme(p.foreground_on_accent, p.urgent.fill, p.urgent.detail),
            },
            hover: WindowColorSet {
                focus: scheme(p.foreground, p.surface_hover, p.primary.hover_detail),
                normal: scheme(p.foreground, p.background_hover, p.background_hover),
                minimized: scheme(p.surface_hover, p.background, p.background),
                sticky: scheme(
                    p.foreground_on_accent,
                    p.special.hover_fill,
                    p.special.hover_detail,
                ),
                sticky_focus: scheme(
                    p.foreground_on_accent,
                    p.focused.hover_fill,
                    p.focused.hover_detail,
                ),
                edge_scratchpad: scheme(
                    p.foreground_on_accent,
                    p.special.hover_fill,
                    p.special.hover_detail,
                ),
                edge_scratchpad_focus: scheme(
                    p.foreground_on_accent,
                    p.focused.hover_fill,
                    p.focused.hover_detail,
                ),
                urgent: scheme(
                    p.foreground_on_accent,
                    p.urgent.hover_fill,
                    p.urgent.hover_detail,
                ),
            },
        },
        close_button: CloseButtonColorConfigs {
            no_hover: CloseButtonColorSet {
                normal: scheme(p.foreground, p.urgent.fill, p.urgent.detail),
                locked: scheme(p.foreground, p.special.fill, p.special.detail),
                fullscreen: scheme(p.foreground, p.urgent.fill, p.urgent.detail),
            },
            hover: CloseButtonColorSet {
                normal: scheme(p.foreground, p.urgent.hover_fill, p.urgent.hover_detail),
                locked: scheme(p.foreground, p.special.hover_fill, p.special.hover_detail),
                fullscreen: scheme(p.foreground, p.urgent.hover_fill, p.urgent.hover_detail),
            },
        },
        border: BorderColorConfig {
            normal: p.surface,
            tile_focus: p.primary.fill,
            float_focus: p.focused.fill,
            snap: p.special.fill,
        },
        status: StatusColorConfig {
            fg: p.foreground,
            bg: p.background,
            detail: p.background,
        },
    }
}

pub fn get_tag_colors() -> TagColorConfigs {
    colors(ColorTheme::Instantos).tag
}
pub fn get_window_colors() -> WindowColorConfigs {
    colors(ColorTheme::Instantos).window
}
pub fn get_close_button_colors() -> CloseButtonColorConfigs {
    colors(ColorTheme::Instantos).close_button
}
pub fn get_border_colors() -> BorderColorConfig {
    colors(ColorTheme::Instantos).border
}
pub fn get_status_bar_colors() -> StatusColorConfig {
    colors(ColorTheme::Instantos).status
}

pub fn get_fonts() -> Vec<String> {
    vec![
        "Inter-Regular:size=12".into(),
        "Fira Code Nerd Font:size=12".into(),
    ]
}
