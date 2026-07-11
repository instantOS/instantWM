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
struct Palette {
    bg: Rgba,
    text: Rgba,
    black: Rgba,
    surface: Rgba,
    surface_hover: Rgba,
    bg_hover: Rgba,
    blue: Rgba,
    blue_hover: Rgba,
    green: Rgba,
    green_hover: Rgba,
    yellow: Rgba,
    yellow_hover: Rgba,
    red: Rgba,
    red_hover: Rgba,
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

fn palette(theme: ColorTheme) -> Palette {
    let values = match theme {
        ColorTheme::Instantos => [
            "121212", "dfdfdf", "000000", "384252", "4c5564", "1c1c1c", "89b3f7", "a1c2f9",
            "81c995", "99d3aa", "fdd663", "fddd82", "f28b82", "f4a19a",
        ],
        ColorTheme::CatppuccinLatte => [
            "eff1f5", "4c4f69", "dce0e8", "ccd0da", "bcc0cc", "e6e9ef", "1e66f5", "7287fd",
            "40a02b", "70b433", "df8e1d", "fead3d", "d20f39", "e64553",
        ],
        ColorTheme::CatppuccinFrappe => [
            "303446", "c6d0f5", "232634", "414559", "51576d", "292c3c", "8caaee", "babbf1",
            "a6d189", "b5d49a", "e5c890", "efcf8e", "e78284", "ea999c",
        ],
        ColorTheme::CatppuccinMacchiato => [
            "24273a", "cad3f5", "181926", "363a4f", "494d64", "1e2030", "8aadf4", "b7bdf8",
            "a6da95", "b8df9f", "eed49f", "f5dcaa", "ed8796", "f49da6",
        ],
        ColorTheme::CatppuccinMocha => [
            "1e1e2e", "cdd6f4", "11111b", "313244", "45475a", "181825", "89b4fa", "b4befe",
            "a6e3a1", "b9e6b5", "f9e2af", "f5e0b5", "f38ba8", "f5a2b8",
        ],
        ColorTheme::Nord => [
            "2e3440", "eceff4", "242933", "3b4252", "4c566a", "343b49", "81a1c1", "88c0d0",
            "a3be8c", "b1c89d", "ebcb8b", "efd49f", "bf616a", "cf7b83",
        ],
        ColorTheme::Gruvbox => [
            "282828", "ebdbb2", "1d2021", "3c3836", "504945", "32302f", "83a598", "8ec07c",
            "b8bb26", "98971a", "fabd2f", "d79921", "fb4934", "cc241d",
        ],
    };
    Palette {
        bg: hex(values[0]),
        text: hex(values[1]),
        black: hex(values[2]),
        surface: hex(values[3]),
        surface_hover: hex(values[4]),
        bg_hover: hex(values[5]),
        blue: hex(values[6]),
        blue_hover: hex(values[7]),
        green: hex(values[8]),
        green_hover: hex(values[9]),
        yellow: hex(values[10]),
        yellow_hover: hex(values[11]),
        red: hex(values[12]),
        red_hover: hex(values[13]),
    }
}

/// Return every resolved colour table for a built-in theme.
pub fn colors(theme: ColorTheme) -> ColorConfig {
    let p = palette(theme);
    let scheme = ColorSchemeRgba::new;
    let mut colors = ColorConfig {
        tag: TagColorConfigs {
            no_hover: TagColorSet {
                inactive: scheme(p.text, p.bg, p.bg),
                filled: scheme(p.text, p.surface, p.blue),
                focus: scheme(p.black, p.green, p.green),
                nofocus: scheme(p.black, p.yellow, p.yellow),
                empty: scheme(p.black, p.red, p.red),
                urgent: scheme(p.black, p.red, p.red),
            },
            hover: TagColorSet {
                inactive: scheme(p.text, p.bg_hover, p.bg),
                filled: scheme(p.text, p.surface_hover, p.blue_hover),
                focus: scheme(p.black, p.green_hover, p.green_hover),
                nofocus: scheme(p.black, p.yellow_hover, p.yellow_hover),
                empty: scheme(p.black, p.red_hover, p.red_hover),
                urgent: scheme(p.black, p.red_hover, p.red_hover),
            },
        },
        window: WindowColorConfigs {
            no_hover: WindowColorSet {
                focus: scheme(p.text, p.surface, p.blue),
                normal: scheme(p.text, p.bg, p.bg),
                minimized: scheme(p.surface, p.bg, p.bg),
                sticky: scheme(p.black, p.yellow, p.yellow),
                sticky_focus: scheme(p.black, p.green, p.green),
                edge_scratchpad: scheme(p.black, p.yellow, p.yellow),
                edge_scratchpad_focus: scheme(p.black, p.green, p.green),
                urgent: scheme(p.black, p.red, p.red),
            },
            hover: WindowColorSet {
                focus: scheme(p.text, p.surface_hover, p.blue_hover),
                normal: scheme(p.text, p.bg_hover, p.bg_hover),
                minimized: scheme(p.surface_hover, p.bg, p.bg),
                sticky: scheme(p.black, p.yellow_hover, p.yellow_hover),
                sticky_focus: scheme(p.black, p.green_hover, p.green_hover),
                edge_scratchpad: scheme(p.black, p.yellow_hover, p.yellow_hover),
                edge_scratchpad_focus: scheme(p.black, p.green_hover, p.green_hover),
                urgent: scheme(p.black, p.red_hover, p.red_hover),
            },
        },
        close_button: CloseButtonColorConfigs {
            no_hover: CloseButtonColorSet {
                normal: scheme(p.text, p.red, p.red),
                locked: scheme(p.text, p.yellow, p.yellow),
                fullscreen: scheme(p.text, p.red, p.red),
            },
            hover: CloseButtonColorSet {
                normal: scheme(p.text, p.red_hover, p.red_hover),
                locked: scheme(p.text, p.yellow_hover, p.yellow_hover),
                fullscreen: scheme(p.text, p.red_hover, p.red_hover),
            },
        },
        border: BorderColorConfig {
            normal: p.surface,
            tile_focus: p.blue,
            float_focus: p.green,
            snap: p.yellow,
        },
        status: StatusColorConfig {
            fg: p.text,
            bg: p.bg,
            detail: p.bg,
        },
    };
    if theme == ColorTheme::Instantos {
        let blue = hex("536dfe");
        let blue_hover = hex("758afe");
        let green = hex("1e8e3e");
        let green_hover = hex("4ba465");
        let yellow = hex("f9ab00");
        let yellow_hover = hex("f9bb33");
        let red = hex("d93025");
        let red_hover = hex("e05951");
        colors.tag.no_hover.filled.detail = blue;
        colors.tag.no_hover.focus.detail = green;
        colors.tag.no_hover.nofocus.detail = yellow;
        colors.tag.no_hover.empty.detail = red;
        colors.tag.no_hover.urgent.detail = red;
        colors.tag.hover.filled.detail = blue_hover;
        colors.tag.hover.focus.detail = green_hover;
        colors.tag.hover.nofocus.detail = yellow_hover;
        colors.tag.hover.empty.detail = red_hover;
        colors.tag.hover.urgent.detail = red_hover;
        colors.window.no_hover.focus.detail = blue;
        colors.window.no_hover.sticky.detail = yellow;
        colors.window.no_hover.sticky_focus.detail = green;
        colors.window.no_hover.edge_scratchpad.detail = yellow;
        colors.window.no_hover.edge_scratchpad_focus.detail = green;
        colors.window.no_hover.urgent.detail = red;
        colors.window.hover.focus.detail = blue_hover;
        colors.window.hover.sticky.detail = yellow_hover;
        colors.window.hover.sticky_focus.detail = green_hover;
        colors.window.hover.edge_scratchpad.detail = yellow_hover;
        colors.window.hover.edge_scratchpad_focus.detail = green_hover;
        colors.window.hover.urgent.detail = red_hover;
        colors.close_button.no_hover.normal.detail = red;
        colors.close_button.no_hover.locked.detail = yellow;
        colors.close_button.no_hover.fullscreen.detail = red;
        colors.close_button.hover.normal.detail = red_hover;
        colors.close_button.hover.locked.detail = yellow_hover;
        colors.close_button.hover.fullscreen.detail = red_hover;
    }
    colors
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
