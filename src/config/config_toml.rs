use crate::config::appearance::{
    get_border_colors, get_close_button_colors, get_fonts, get_status_bar_colors, get_tag_colors,
    get_window_colors,
};
use crate::config::keybind_config::KeybindSpec;
use crate::types::{
    BorderColorConfig, CloseButtonColorConfigs, StatusColorConfig, TagColorConfigs,
    WindowColorConfigs,
};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub fonts: Vec<String>,
    pub colors: ColorConfig,
    /// User-defined keybinds (override/extend defaults).
    pub keybinds: Vec<KeybindSpec>,
    /// User-defined desktop keybinds (override/extend defaults).
    pub desktop_keybinds: Vec<KeybindSpec>,
    /// Keyboard layout configuration.
    pub keyboard: KeyboardConfig,
    /// Input configuration (mouse, touchpad).
    pub input: std::collections::HashMap<String, InputConfig>,
    /// Background command to execute for reading status bar text, typically `i3status-rs`
    pub status_command: Option<String>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            fonts: get_fonts(),
            colors: ColorConfig::default(),
            keybinds: Vec::new(),
            desktop_keybinds: Vec::new(),
            keyboard: KeyboardConfig::default(),
            input: std::collections::HashMap::new(),
            status_command: None,
        }
    }
}

/// Toggle setting for boolean-like input options (tap, natural_scroll).
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToggleSetting {
    Enabled,
    Disabled,
}

/// Acceleration profile for pointer devices.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AccelProfile {
    Flat,
    Adaptive,
}

/// Input configuration from the TOML `[input]` section.
/// Allows per-device or type-based (like `type:touchpad`) configuration
/// similar to Sway.
#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(default)]
pub struct InputConfig {
    pub tap: Option<ToggleSetting>,
    pub natural_scroll: Option<ToggleSetting>,
    pub accel_profile: Option<AccelProfile>,
    pub pointer_accel: Option<f64>,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            tap: None,
            natural_scroll: None,
            accel_profile: None,
            pointer_accel: None,
        }
    }
}

/// Keyboard (XKB) layout configuration from the TOML `[keyboard]` section.
///
/// ```toml
/// [keyboard]
/// layouts = [
///   { name = "us" },
///   { name = "de", variant = "nodeadkeys" },
///   { name = "fr" }
/// ]
/// options = "grp:alt_shift_toggle"
/// ```
#[derive(Debug, Deserialize, Clone, Serialize, Default)]
#[serde(default)]
pub struct KeyboardLayoutConfig {
    /// Layout name (e.g., "us", "de", "fr").
    pub name: String,
    /// Optional variant (e.g., "nodeadkeys", "colemak").
    #[serde(default)]
    pub variant: Option<String>,
}

/// Keyboard (XKB) layout configuration from the TOML `[keyboard]` section.
#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(default)]
pub struct KeyboardConfig {
    /// XKB layout configurations.
    #[serde(default)]
    pub layouts: Vec<KeyboardLayoutConfig>,
    /// XKB options string, e.g. `"grp:alt_shift_toggle,compose:ralt"`.
    pub options: Option<String>,
    /// XKB model, e.g. `"pc105"`. Defaults to system default if unset.
    pub model: Option<String>,
}

impl Default for KeyboardConfig {
    fn default() -> Self {
        Self {
            layouts: Vec::new(),
            options: None,
            model: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
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
