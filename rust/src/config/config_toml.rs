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

impl TagColorSetToml {
    fn triplet(&self, scheme: SchemeTag) -> Option<&ColorTriplet> {
        match scheme {
            SchemeTag::Inactive => self.inactive.as_ref(),
            SchemeTag::Filled => self.filled.as_ref(),
            SchemeTag::Focus => self.focus.as_ref(),
            SchemeTag::NoFocus => self.nofocus.as_ref(),
            SchemeTag::Empty => self.empty.as_ref(),
        }
    }
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

impl WindowColorSetToml {
    fn triplet(&self, scheme: SchemeWin) -> Option<&ColorTriplet> {
        match scheme {
            SchemeWin::Focus => self.focus.as_ref(),
            SchemeWin::Normal => self.normal.as_ref(),
            SchemeWin::Minimized => self.minimized.as_ref(),
            SchemeWin::Sticky => self.sticky.as_ref(),
            SchemeWin::StickyFocus => self.sticky_focus.as_ref(),
            SchemeWin::Overlay => self.overlay.as_ref(),
            SchemeWin::OverlayFocus => self.overlay_focus.as_ref(),
        }
    }
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

impl CloseButtonColorSetToml {
    fn triplet(&self, scheme: SchemeClose) -> Option<&ColorTriplet> {
        match scheme {
            SchemeClose::Normal => self.normal.as_ref(),
            SchemeClose::Locked => self.locked.as_ref(),
            SchemeClose::Fullscreen => self.fullscreen.as_ref(),
        }
    }
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
    const TAG_SCHEMES: [SchemeTag; 5] = [
        SchemeTag::Inactive,
        SchemeTag::Filled,
        SchemeTag::Focus,
        SchemeTag::NoFocus,
        SchemeTag::Empty,
    ];

    for scheme in TAG_SCHEMES {
        if let Some(triplet) = src.triplet(scheme) {
            apply_triplet(target.scheme_mut(scheme), triplet);
        }
    }
}

fn apply_window_set(target: &mut crate::types::WindowColorSet, src: &WindowColorSetToml) {
    const WINDOW_SCHEMES: [SchemeWin; 7] = [
        SchemeWin::Focus,
        SchemeWin::Normal,
        SchemeWin::Minimized,
        SchemeWin::Sticky,
        SchemeWin::StickyFocus,
        SchemeWin::Overlay,
        SchemeWin::OverlayFocus,
    ];

    for scheme in WINDOW_SCHEMES {
        if let Some(triplet) = src.triplet(scheme) {
            apply_triplet(target.scheme_mut(scheme), triplet);
        }
    }
}

fn apply_close_button_set(
    target: &mut crate::types::CloseButtonColorSet,
    src: &CloseButtonColorSetToml,
) {
    const CLOSE_SCHEMES: [SchemeClose; 3] = [
        SchemeClose::Normal,
        SchemeClose::Locked,
        SchemeClose::Fullscreen,
    ];

    for scheme in CLOSE_SCHEMES {
        if let Some(triplet) = src.triplet(scheme) {
            apply_triplet(target.scheme_mut(scheme), triplet);
        }
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
