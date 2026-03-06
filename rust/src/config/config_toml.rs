use crate::config::Config;
use crate::types::{ColIndex, SchemeBorder, SchemeClose, SchemeTag, SchemeWin};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Default)]
struct ConfigToml {
    pub fonts: Option<Vec<String>>,
    pub colors: Option<ColorConfigToml>,
}

#[derive(Debug, Deserialize, Default)]
struct ColorConfigToml {
    pub tag: Option<TagColorToml>,
    pub window: Option<WindowColorToml>,
    pub close_button: Option<CloseButtonColorToml>,
    pub border: Option<BorderColorToml>,
    pub status: Option<StatusColorToml>,
}

#[derive(Debug, Deserialize, Default)]
struct TagColorToml {
    pub normal: Option<TagColorSetToml>,
    pub hover: Option<TagColorSetToml>,
}

#[derive(Debug, Deserialize, Default)]
struct TagColorSetToml {
    pub inactive: Option<ColorTriplet>,
    pub filled: Option<ColorTriplet>,
    pub focus: Option<ColorTriplet>,
    pub nofocus: Option<ColorTriplet>,
    pub empty: Option<ColorTriplet>,
}

#[derive(Debug, Deserialize, Default)]
struct WindowColorToml {
    pub normal: Option<WindowColorSetToml>,
    pub hover: Option<WindowColorSetToml>,
}

#[derive(Debug, Deserialize, Default)]
struct WindowColorSetToml {
    pub focus: Option<ColorTriplet>,
    pub normal: Option<ColorTriplet>,
    pub minimized: Option<ColorTriplet>,
    pub sticky: Option<ColorTriplet>,
    pub sticky_focus: Option<ColorTriplet>,
    pub overlay: Option<ColorTriplet>,
    pub overlay_focus: Option<ColorTriplet>,
}

#[derive(Debug, Deserialize, Default)]
struct CloseButtonColorToml {
    pub normal: Option<CloseButtonColorSetToml>,
    pub hover: Option<CloseButtonColorSetToml>,
}

#[derive(Debug, Deserialize, Default)]
struct CloseButtonColorSetToml {
    pub normal: Option<ColorTriplet>,
    pub locked: Option<ColorTriplet>,
    pub fullscreen: Option<ColorTriplet>,
}

#[derive(Debug, Deserialize, Default)]
struct BorderColorToml {
    pub normal: Option<String>,
    pub tile_focus: Option<String>,
    pub float_focus: Option<String>,
    pub snap: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct StatusColorToml {
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ColorTriplet {
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub detail: Option<String>,
}

fn apply_triplet(target: &mut crate::types::ColorSchemeStrings, src: &ColorTriplet) {
    if let Some(value) = &src.fg {
        target.set(ColIndex::Fg, value.clone());
    }
    if let Some(value) = &src.bg {
        target.set(ColIndex::Bg, value.clone());
    }
    if let Some(value) = &src.detail {
        target.set(ColIndex::Detail, value.clone());
    }
}

fn apply_tag_set(target: &mut crate::types::TagColorSet, src: &TagColorSetToml) {
    if let Some(triplet) = &src.inactive {
        apply_triplet(target.scheme_mut(SchemeTag::Inactive), triplet);
    }
    if let Some(triplet) = &src.filled {
        apply_triplet(target.scheme_mut(SchemeTag::Filled), triplet);
    }
    if let Some(triplet) = &src.focus {
        apply_triplet(target.scheme_mut(SchemeTag::Focus), triplet);
    }
    if let Some(triplet) = &src.nofocus {
        apply_triplet(target.scheme_mut(SchemeTag::NoFocus), triplet);
    }
    if let Some(triplet) = &src.empty {
        apply_triplet(target.scheme_mut(SchemeTag::Empty), triplet);
    }
}

fn apply_window_set(target: &mut crate::types::WindowColorSet, src: &WindowColorSetToml) {
    if let Some(triplet) = &src.focus {
        apply_triplet(target.scheme_mut(SchemeWin::Focus), triplet);
    }
    if let Some(triplet) = &src.normal {
        apply_triplet(target.scheme_mut(SchemeWin::Normal), triplet);
    }
    if let Some(triplet) = &src.minimized {
        apply_triplet(target.scheme_mut(SchemeWin::Minimized), triplet);
    }
    if let Some(triplet) = &src.sticky {
        apply_triplet(target.scheme_mut(SchemeWin::Sticky), triplet);
    }
    if let Some(triplet) = &src.sticky_focus {
        apply_triplet(target.scheme_mut(SchemeWin::StickyFocus), triplet);
    }
    if let Some(triplet) = &src.overlay {
        apply_triplet(target.scheme_mut(SchemeWin::Overlay), triplet);
    }
    if let Some(triplet) = &src.overlay_focus {
        apply_triplet(target.scheme_mut(SchemeWin::OverlayFocus), triplet);
    }
}

fn apply_close_button_set(
    target: &mut crate::types::CloseButtonColorSet,
    src: &CloseButtonColorSetToml,
) {
    if let Some(triplet) = &src.normal {
        apply_triplet(target.scheme_mut(SchemeClose::Normal), triplet);
    }
    if let Some(triplet) = &src.locked {
        apply_triplet(target.scheme_mut(SchemeClose::Locked), triplet);
    }
    if let Some(triplet) = &src.fullscreen {
        apply_triplet(target.scheme_mut(SchemeClose::Fullscreen), triplet);
    }
}

pub fn default_config_path() -> Option<PathBuf> {
    let config_dir = dirs::config_dir()?;
    Some(config_dir.join("instantwm").join("config.toml"))
}

fn apply_colors(parsed: &ConfigToml, cfg: &mut Config) {
    let Some(colors) = &parsed.colors else {
        return;
    };

    if let Some(tag) = &colors.tag {
        if let Some(set) = &tag.normal {
            apply_tag_set(&mut cfg.tag_colors.no_hover, set);
        }
        if let Some(set) = &tag.hover {
            apply_tag_set(&mut cfg.tag_colors.hover, set);
        }
    }

    if let Some(window) = &colors.window {
        if let Some(set) = &window.normal {
            apply_window_set(&mut cfg.windowcolors.no_hover, set);
        }
        if let Some(set) = &window.hover {
            apply_window_set(&mut cfg.windowcolors.hover, set);
        }
    }

    if let Some(close_button) = &colors.close_button {
        if let Some(set) = &close_button.normal {
            apply_close_button_set(&mut cfg.closebuttoncolors.no_hover, set);
        }
        if let Some(set) = &close_button.hover {
            apply_close_button_set(&mut cfg.closebuttoncolors.hover, set);
        }
    }

    if let Some(border) = &colors.border {
        if let Some(value) = &border.normal {
            cfg.bordercolors.set(SchemeBorder::Normal, value.clone());
        }
        if let Some(value) = &border.tile_focus {
            cfg.bordercolors.set(SchemeBorder::TileFocus, value.clone());
        }
        if let Some(value) = &border.float_focus {
            cfg.bordercolors
                .set(SchemeBorder::FloatFocus, value.clone());
        }
        if let Some(value) = &border.snap {
            cfg.bordercolors.set(SchemeBorder::Snap, value.clone());
        }
    }

    if let Some(status) = &colors.status {
        if let Some(value) = &status.fg {
            cfg.statusbarcolors.set(ColIndex::Fg, value.clone());
        }
        if let Some(value) = &status.bg {
            cfg.statusbarcolors.set(ColIndex::Bg, value.clone());
        }
        if let Some(value) = &status.detail {
            cfg.statusbarcolors.set(ColIndex::Detail, value.clone());
        }
    }
}

pub fn load_config_toml(path: &Path) -> Result<ConfigToml, String> {
    let contents = fs::read_to_string(path).map_err(|err| err.to_string())?;
    toml::from_str(&contents).map_err(|err| err.to_string())
}

pub fn apply_config_overrides(cfg: &mut Config) -> Result<(), String> {
    let Some(path) = default_config_path() else {
        return Ok(());
    };

    if !path.exists() {
        return Ok(());
    }

    let parsed = load_config_toml(&path)?;

    if let Some(fonts) = parsed.fonts.clone() {
        if !fonts.is_empty() {
            cfg.fonts = fonts;
        }
    }

    apply_colors(&parsed, cfg);
    Ok(())
}
