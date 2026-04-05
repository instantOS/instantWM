use crate::config::appearance::{
    get_border_colors, get_close_button_colors, get_fonts, get_status_bar_colors, get_tag_colors,
    get_window_colors,
};
use crate::config::keybind_config::KeybindSpec;
use crate::types::{
    BorderColorConfig, CloseButtonColorConfigs, Rule, StatusColorConfig, TagColorConfigs,
    WindowColorConfigs,
};
use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct IncludeConfig {
    pub file: String,
}

/// Mode specification for sway-like modes.
#[derive(Debug, Deserialize, Clone, Serialize, Default)]
pub struct ModeSpec {
    /// Optional description shown in status bar when mode is active.
    #[serde(default)]
    pub description: Option<String>,
    /// Whether the mode is transient (reset to default after any keybind).
    pub transient: Option<bool>,
    /// Keybinds for this mode.
    #[serde(default)]
    pub keybinds: Vec<KeybindSpec>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub includes: Vec<IncludeConfig>,
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
    /// Monitor configuration.
    pub monitors: std::collections::HashMap<String, MonitorConfig>,
    /// Background command to execute for reading status bar text, typically `i3status-rs`
    pub status_command: Option<String>,
    /// User-defined modes (sway-like modes).
    pub modes: std::collections::HashMap<String, ModeSpec>,
    /// Cursor configuration (Wayland only).
    pub cursor: CursorConfig,
    /// Window rules.
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            includes: Vec::new(),
            fonts: get_fonts(),
            colors: ColorConfig::default(),
            keybinds: Vec::new(),
            desktop_keybinds: Vec::new(),
            keyboard: KeyboardConfig::default(),
            input: std::collections::HashMap::new(),
            monitors: std::collections::HashMap::new(),
            status_command: None,
            modes: std::collections::HashMap::new(),
            cursor: CursorConfig::default(),
            rules: Vec::new(),
        }
    }
}

/// Cursor configuration for Wayland.
#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(default)]
pub struct CursorConfig {
    pub theme: String,
    pub size: u32,
}

impl Default for CursorConfig {
    fn default() -> Self {
        Self {
            theme: "Adwaita".to_string(),
            size: 24,
        }
    }
}

/// Monitor configuration from the TOML `[monitors]` section.
#[derive(Debug, Deserialize, Clone, Serialize, Default)]
#[serde(default)]
pub struct MonitorConfig {
    /// Resolution in "WIDTHxHEIGHT" format (e.g., "1920x1080").
    pub resolution: Option<String>,
    /// Refresh rate in Hz (e.g., 60.0).
    pub refresh_rate: Option<f32>,
    /// Position in "X,Y" format (e.g., "1920,0") or relative (e.g., "left-of:DP-1").
    pub position: Option<String>,
    /// Scale factor (e.g., 1.0, 2.0).
    pub scale: Option<f32>,
    /// Transform (e.g., "normal", "90", "180", "270", "flipped", "flipped-90", "flipped-180", "flipped-270").
    pub transform: Option<String>,
    /// Whether the monitor is enabled.
    pub enable: Option<bool>,
    /// Variable refresh rate policy for this output.
    pub vrr: Option<VrrMode>,
}

#[derive(
    Debug,
    Deserialize,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Default,
    Encode,
    Decode,
    clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum VrrMode {
    Off,
    #[default]
    Auto,
    On,
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
    pub scroll_factor: Option<f64>,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            tap: Some(ToggleSetting::Enabled),
            natural_scroll: None,
            accel_profile: None,
            pointer_accel: None,
            scroll_factor: None,
        }
    }
}

/// Keyboard (XKB) layout configuration from the TOML `[keyboard]` section.
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
///
/// ```toml
/// [keyboard]
/// layouts = [
///   { name = "us" },
///   { name = "de", variant = "nodeadkeys" },
///   { name = "fr" }
/// ]
/// options = "grp:alt_shift_toggle"
/// swapescape = true
/// ```
#[derive(Debug, Deserialize, Clone, Serialize, Default)]
#[serde(default)]
pub struct KeyboardConfig {
    /// XKB layout configurations.
    #[serde(default)]
    pub layouts: Vec<KeyboardLayoutConfig>,
    /// XKB options string, e.g. `"grp:alt_shift_toggle,compose:ralt"`.
    pub options: Option<String>,
    /// XKB model, e.g. `"pc105"`. Defaults to system default if unset.
    pub model: Option<String>,
    /// Swap Caps Lock and Escape.
    pub swapescape: bool,
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

    let mut visited = HashSet::new();
    match load_and_merge_config(&path, &mut visited) {
        Ok(merged_value) => match merged_value.try_into::<ThemeConfig>() {
            Ok(file) => file,
            Err(e) => {
                eprintln!("instantwm: config parse error, using defaults: {e}");
                ThemeConfig::default()
            }
        },
        Err(_) => ThemeConfig::default(),
    }
}

fn load_and_merge_config(path: &Path, visited: &mut HashSet<PathBuf>) -> Result<toml::Value, ()> {
    let canonical_path = path.canonicalize().map_err(|e| {
        eprintln!("instantwm: could not canonicalize path {:?}: {e}", path);
    })?;

    if visited.contains(&canonical_path) {
        eprintln!("instantwm: circular include detected: {:?}", canonical_path);
        return Err(());
    }
    visited.insert(canonical_path.clone());

    let contents = fs::read_to_string(path).map_err(|e| {
        eprintln!("instantwm: could not read config file {:?}: {e}", path);
    })?;

    let value: toml::Value = toml::from_str(&contents).map_err(|e| {
        eprintln!("instantwm: config parse error in {:?}: {e}", path);
    })?;

    let mut merged_base = toml::Value::Table(toml::Table::new());

    if let Some(includes) = value.get("includes").and_then(|v| v.as_array()) {
        let parent_dir = path.parent().unwrap_or(Path::new("."));

        for include in includes {
            if let Some(file_path_str) = include.get("file").and_then(|v| v.as_str()) {
                let include_path = if Path::new(file_path_str).is_absolute() {
                    PathBuf::from(file_path_str)
                } else {
                    parent_dir.join(file_path_str)
                };

                if !include_path.exists() {
                    eprintln!(
                        "instantwm: warning: included config file {:?} does not exist",
                        include_path
                    );
                    continue;
                }

                if let Ok(included_value) = load_and_merge_config(&include_path, visited) {
                    merge_toml_values(&mut merged_base, included_value);
                }
            }
        }
    }

    // Merge current file OVER includes
    merge_toml_values(&mut merged_base, value);

    Ok(merged_base)
}

fn merge_toml_values(base: &mut toml::Value, over: toml::Value) {
    match (base, over) {
        (toml::Value::Table(base_table), toml::Value::Table(over_table)) => {
            for (key, value) in over_table {
                if key == "includes" {
                    if let Some(base_includes) = base_table.get_mut("includes") {
                        if let (toml::Value::Array(base_arr), toml::Value::Array(over_arr)) =
                            (base_includes, value)
                        {
                            base_arr.extend(over_arr);
                        }
                    } else {
                        base_table.insert(key, value);
                    }
                    continue;
                }

                if let Some(base_value) = base_table.get_mut(&key) {
                    merge_toml_values(base_value, value);
                } else {
                    base_table.insert(key, value);
                }
            }
        }
        (toml::Value::Array(base_array), toml::Value::Array(over_array)) => {
            base_array.extend(over_array);
        }
        (base, over) => {
            *base = over;
        }
    }
}
