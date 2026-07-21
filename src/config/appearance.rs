//! Built-in colour themes and font configuration.
//!
//! Backends consume the resolved, typed colour tables. Theme selection and
//! user overrides are handled while loading TOML, before runtime state exists.

use crate::bar::color::Rgba;
use crate::config::config_toml::{ColorConfig, ColorTheme};
use crate::types::{
    BorderColorConfig, CloseButtonColorConfigs, CloseButtonColorSet, ColorSchemeRgba,
    StatusColorConfig, TagColorConfigs, TagColorSet, WindowColorConfigs, WindowColorSet,
};

#[derive(Clone, Copy)]
struct Accent {
    fill: Rgba,
    detail: Rgba,
    hover_fill: Rgba,
    hover_detail: Rgba,
}

impl Accent {
    /// Build an accent whose detail is a darker, muted version of its fill.
    ///
    /// Detail is used for the lower edge of buttons and title blocks, so using
    /// the fill verbatim makes that edge disappear. Mixing with a neutral tone
    /// from the theme also avoids introducing an unrelated saturated colour.
    const fn shaded(fill: &str, hover_fill: &str, shadow: &str) -> Self {
        let fill = hex(fill);
        let hover_fill = hex(hover_fill);
        let shadow = hex(shadow);
        Self {
            fill,
            detail: mix(fill, shadow, 0.25),
            hover_fill,
            hover_detail: mix(hover_fill, shadow, 0.25),
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
    Rgba::rgb(
        (digit(b[i]) * 16 + digit(b[i + 1])) as f32 / 255.0,
        (digit(b[i + 2]) * 16 + digit(b[i + 3])) as f32 / 255.0,
        (digit(b[i + 4]) * 16 + digit(b[i + 5])) as f32 / 255.0,
    )
}

const fn mix(color: Rgba, other: Rgba, other_weight: f32) -> Rgba {
    let cw = 1.0 - other_weight;
    Rgba::new(
        color.r() * cw + other.r() * other_weight,
        color.g() * cw + other.g() * other_weight,
        color.b() * cw + other.b() * other_weight,
        color.a() * cw + other.a() * other_weight,
    )
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
            primary: Accent::shaded("89b3f7", "a1c2f9", "121212"),
            focused: Accent::shaded("81c995", "99d3aa", "121212"),
            special: Accent::shaded("fdd663", "fddd82", "121212"),
            urgent: Accent::shaded("f28b82", "f4a19a", "121212"),
        },
        ColorTheme::CatppuccinLatte => ThemePalette {
            background: hex("eff1f5"),
            foreground: hex("4c4f69"),
            foreground_on_accent: hex("dce0e8"),
            background_hover: hex("e6e9ef"),
            surface: hex("ccd0da"),
            surface_hover: hex("bcc0cc"),
            primary: Accent::shaded("1e66f5", "7287fd", "4c4f69"),
            focused: Accent::shaded("40a02b", "70b433", "4c4f69"),
            special: Accent::shaded("df8e1d", "fead3d", "4c4f69"),
            urgent: Accent::shaded("d20f39", "e64553", "4c4f69"),
        },
        ColorTheme::CatppuccinFrappe => ThemePalette {
            background: hex("303446"),
            foreground: hex("c6d0f5"),
            foreground_on_accent: hex("232634"),
            background_hover: hex("292c3c"),
            surface: hex("414559"),
            surface_hover: hex("51576d"),
            primary: Accent::shaded("8caaee", "babbf1", "232634"),
            focused: Accent::shaded("a6d189", "b5d49a", "232634"),
            special: Accent::shaded("e5c890", "efcf8e", "232634"),
            urgent: Accent::shaded("e78284", "ea999c", "232634"),
        },
        ColorTheme::CatppuccinMacchiato => ThemePalette {
            background: hex("24273a"),
            foreground: hex("cad3f5"),
            foreground_on_accent: hex("181926"),
            background_hover: hex("1e2030"),
            surface: hex("363a4f"),
            surface_hover: hex("494d64"),
            primary: Accent::shaded("8aadf4", "b7bdf8", "181926"),
            focused: Accent::shaded("a6da95", "b8df9f", "181926"),
            special: Accent::shaded("eed49f", "f5dcaa", "181926"),
            urgent: Accent::shaded("ed8796", "f49da6", "181926"),
        },
        ColorTheme::CatppuccinMocha => ThemePalette {
            background: hex("1e1e2e"),
            foreground: hex("cdd6f4"),
            foreground_on_accent: hex("11111b"),
            background_hover: hex("181825"),
            surface: hex("313244"),
            surface_hover: hex("45475a"),
            primary: Accent::shaded("89b4fa", "b4befe", "11111b"),
            focused: Accent::shaded("a6e3a1", "b9e6b5", "11111b"),
            special: Accent::shaded("f9e2af", "f5e0b5", "11111b"),
            urgent: Accent::shaded("f38ba8", "f5a2b8", "11111b"),
        },
        ColorTheme::Nord => ThemePalette {
            background: hex("2e3440"),
            foreground: hex("eceff4"),
            foreground_on_accent: hex("242933"),
            background_hover: hex("343b49"),
            surface: hex("3b4252"),
            surface_hover: hex("4c566a"),
            primary: Accent::shaded("81a1c1", "88c0d0", "242933"),
            focused: Accent::shaded("a3be8c", "b1c89d", "242933"),
            special: Accent::shaded("ebcb8b", "efd49f", "242933"),
            urgent: Accent::shaded("bf616a", "cf7b83", "242933"),
        },
        ColorTheme::Gruvbox => ThemePalette {
            background: hex("282828"),
            foreground: hex("ebdbb2"),
            foreground_on_accent: hex("1d2021"),
            background_hover: hex("32302f"),
            surface: hex("3c3836"),
            surface_hover: hex("504945"),
            primary: Accent::shaded("83a598", "8ec07c", "1d2021"),
            focused: Accent::shaded("b8bb26", "98971a", "1d2021"),
            special: Accent::shaded("fabd2f", "d79921", "1d2021"),
            urgent: Accent::shaded("fb4934", "cc241d", "1d2021"),
        },
    }
}

impl From<ColorTheme> for ColorConfig {
    /// Resolve every colour table for a built-in theme.
    fn from(theme: ColorTheme) -> Self {
        let p = palette(theme);
        let scheme = ColorSchemeRgba::new;
        Self {
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
                    edge_scratchpad: scheme(
                        p.foreground_on_accent,
                        p.special.fill,
                        p.special.detail,
                    ),
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
                hover: p.primary.hover_fill,
            },
        }
    }
}

pub fn get_fonts() -> Vec<String> {
    vec![
        "Inter-Regular:size=12".into(),
        "Fira Code Nerd Font:size=12".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_close_button_has_visible_detail() {
        for theme in ColorTheme::ALL {
            let close = ColorConfig::from(*theme).close_button;
            for set in [&close.no_hover, &close.hover] {
                for scheme in [&set.normal, &set.locked, &set.fullscreen] {
                    assert_ne!(
                        scheme.bg, scheme.detail,
                        "{} close-button detail must contrast with its fill",
                        theme
                    );
                }
            }
        }
    }

    #[test]
    fn every_builtin_status_hover_contrasts_with_the_bar() {
        for theme in ColorTheme::ALL {
            let status = ColorConfig::from(*theme).status;
            assert_ne!(
                status.hover, status.bg,
                "{} status hover must contrast with the bar background",
                theme
            );
        }
    }

    #[test]
    fn instantos_primary_detail_is_a_muted_shadow() {
        let primary = palette(ColorTheme::Instantos).primary;
        assert_ne!(primary.fill, primary.detail);
        assert!(primary.detail.r() < primary.fill.r());
        assert!(primary.detail.g() < primary.fill.g());
        assert!(primary.detail.b() < primary.fill.b());
    }
}
