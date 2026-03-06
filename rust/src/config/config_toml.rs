use crate::config::appearance::{
    get_border_colors, get_close_button_colors, get_fonts, get_status_bar_colors, get_tag_colors,
    get_window_colors,
};
use crate::config::keybind_config::KeybindSpec;
use crate::types::{
    BorderColorConfig, CloseButtonColorConfigs, StatusColorConfig, TagColorConfigs,
    WindowColorConfigs,
};
use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub fonts: Vec<String>,
    pub colors: ColorConfig,
    /// User-defined keybinds (override/extend defaults).
    pub keybinds: Vec<KeybindSpec>,
    /// User-defined desktop keybinds (override/extend defaults).
    pub desktop_keybinds: Vec<KeybindSpec>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            fonts: get_fonts(),
            colors: ColorConfig::default(),
            keybinds: Vec::new(),
            desktop_keybinds: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ColorConfig {
    pub tag: TagColorConfigs,
    pub window: WindowColorConfigs,
    pub close_button: CloseButtonColorConfigs,
    pub border: BorderColorConfig,
    pub status: StatusColorConfig,
}

impl Default for ColorConfig {
    fn default() -> Self {
        Self {
            tag: get_tag_colors(),
            window: get_window_colors(),
            close_button: get_close_button_colors(),
            border: get_border_colors(),
            status: get_status_bar_colors(),
        }
    }
}

pub fn load_config_file() -> ThemeConfig {
    let path = match dirs::config_dir() {
        Some(dir) => dir.join("instantwm").join("config.toml"),
        None => return ThemeConfig::default(),
    };

    if !path.exists() {
        return ThemeConfig::default();
    }

    let contents = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("instantwm: could not read config: {e}");
            return ThemeConfig::default();
        }
    };

    match toml::from_str(&contents) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("instantwm: config parse error, using defaults: {e}");
            ThemeConfig::default()
        }
    }
}
