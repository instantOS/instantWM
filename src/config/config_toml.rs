use crate::config::appearance::get_fonts;
use crate::config::keybind_config::KeybindSpec;
use crate::types::{
    BorderColorConfig, CloseButtonColorConfigs, Rule, StatusColorConfig, TagColorConfigs,
    WindowColorConfigs,
};
use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
    /// Built-in colour theme used as the base for `[colors]` overrides.
    pub theme: ColorTheme,
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
    pub input: HashMap<String, InputConfig>,
    /// Monitor configuration.
    pub monitors: HashMap<String, MonitorConfig>,
    /// Background command to execute for reading status bar text, typically `i3status-rs`
    pub status_command: Option<String>,
    /// User-defined modes (sway-like modes).
    pub modes: HashMap<String, ModeSpec>,
    /// Cursor configuration (Wayland only).
    pub cursor: CursorConfig,
    /// Layout geometry configuration.
    pub layout: LayoutConfig,
    /// Window rules.
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Bar height in logical pixels. 0 = auto (derive from font metrics).
    #[serde(default)]
    pub bar_height: u32,
    /// Commands to execute once at startup (like sway `exec` / Hyprland `exec-once`).
    #[serde(default)]
    pub exec_once: Vec<String>,
    /// Commands to execute at startup and on every config reload (like sway `exec_always`).
    #[serde(default)]
    pub exec: Vec<String>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            theme: ColorTheme::default(),
            includes: Vec::new(),
            fonts: get_fonts(),
            colors: ColorConfig::default(),
            keybinds: Vec::new(),
            desktop_keybinds: Vec::new(),
            keyboard: KeyboardConfig::default(),
            input: HashMap::new(),
            monitors: HashMap::new(),
            status_command: None,
            modes: HashMap::new(),
            cursor: CursorConfig::default(),
            layout: LayoutConfig::default(),
            rules: Vec::new(),
            bar_height: 0,
            exec_once: Vec::new(),
            exec: Vec::new(),
        }
    }
}

/// A built-in base colour theme. Names use kebab-case in TOML.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Decode, Encode)]
#[serde(rename_all = "kebab-case")]
pub enum ColorTheme {
    #[default]
    Instantos,
    CatppuccinLatte,
    CatppuccinFrappe,
    CatppuccinMacchiato,
    CatppuccinMocha,
    Nord,
    Gruvbox,
}

impl ColorTheme {
    /// All built-in themes, in the order shown by `instantwmctl theme --list`.
    pub const ALL: &[ColorTheme] = &[
        ColorTheme::Instantos,
        ColorTheme::CatppuccinLatte,
        ColorTheme::CatppuccinFrappe,
        ColorTheme::CatppuccinMacchiato,
        ColorTheme::CatppuccinMocha,
        ColorTheme::Nord,
        ColorTheme::Gruvbox,
    ];
}

impl std::fmt::Display for ColorTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Keep these spellings aligned with the enum's serde names. `FromStr`
        // delegates to serde, and `display_names_match_serde_names_for_every_variant`
        // exhaustively checks the mapping whenever a variant is added or renamed.
        let name = match self {
            Self::Instantos => "instantos",
            Self::CatppuccinLatte => "catppuccin-latte",
            Self::CatppuccinFrappe => "catppuccin-frappe",
            Self::CatppuccinMacchiato => "catppuccin-macchiato",
            Self::CatppuccinMocha => "catppuccin-mocha",
            Self::Nord => "nord",
            Self::Gruvbox => "gruvbox",
        };
        f.write_str(name)
    }
}

impl std::str::FromStr for ColorTheme {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        toml::Value::String(name.to_string())
            .try_into()
            .map_err(|_| format!("unknown color theme: {name}"))
    }
}

/// Layout geometry configuration.
///
/// ```toml
/// [layout]
/// inner_gap = 8
/// outer_gap = 8
/// smart_gaps = true
/// monocle_gaps = false
/// ```
#[derive(Debug, Deserialize, Clone, Copy, Serialize)]
#[serde(default)]
#[derive(Default)]
pub struct LayoutConfig {
    /// Gap between tiled windows in logical pixels.
    pub inner_gap: i32,
    /// Gap between tiled windows and the monitor work area edge in logical pixels.
    pub outer_gap: i32,
    /// Disable gaps when a tiling layout has one or fewer tiled windows.
    pub smart_gaps: bool,
    /// Apply configured gaps to monocle layout.
    pub monocle_gaps: bool,
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
    pub left_handed: Option<ToggleSetting>,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            tap: Some(ToggleSetting::Enabled),
            natural_scroll: None,
            accel_profile: None,
            pointer_accel: None,
            scroll_factor: None,
            left_handed: None,
        }
    }
}

impl std::fmt::Display for InputConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "tap: {:?}", self.tap)?;
        writeln!(f, "natural_scroll: {:?}", self.natural_scroll)?;
        writeln!(f, "accel_profile: {:?}", self.accel_profile)?;
        writeln!(f, "pointer_accel: {:?}", self.pointer_accel)?;
        writeln!(f, "scroll_factor: {:?}", self.scroll_factor)?;
        write!(f, "left_handed: {:?}", self.left_handed)
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
        ColorTheme::default().into()
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
        Ok(merged_value) => match resolve_theme_colors(merged_value)
            .and_then(|v| v.try_into::<ThemeConfig>().map_err(|e| e.to_string()))
        {
            Ok(file) => file,
            Err(e) => {
                eprintln!("instantwm: config parse error, using defaults: {e}");
                ThemeConfig::default()
            }
        },
        Err(_) => ThemeConfig::default(),
    }
}

fn resolve_theme_colors(mut config: toml::Value) -> Result<toml::Value, String> {
    let theme = match config.get("theme").cloned() {
        None => ColorTheme::default(),
        Some(value) => match value.clone().try_into::<ColorTheme>() {
            Ok(theme) => theme,
            Err(_) => {
                eprintln!("instantwm: unknown theme {value}, falling back to the default theme");
                // Drop the bad key so ThemeConfig deserialisation succeeds;
                // the struct is `#[serde(default)]`, so the field resolves to
                // the default theme and every other setting still loads.
                if let Some(table) = config.as_table_mut() {
                    table.remove("theme");
                }
                ColorTheme::default()
            }
        },
    };
    let mut base = toml::Value::try_from(ColorConfig::from(theme)).map_err(|e| e.to_string())?;
    if let Some(overrides) = config.get_mut("colors") {
        merge_toml_values(
            &mut base,
            std::mem::replace(overrides, toml::Value::Table(toml::Table::new())),
        );
        *overrides = base;
    } else if let Some(table) = config.as_table_mut() {
        table.insert("colors".into(), base);
    }
    Ok(config)
}

/// Generate a commented-out default config template.
///
/// All settings are commented out so that:
/// - Users can see what options are available
/// - Defaults are not baked in, so they track upstream changes
pub fn generate_commented_config() -> String {
    let config = ThemeConfig::default();
    let full = toml::to_string_pretty(&config).expect("failed to serialize default config");

    let mut out = String::new();
    out.push_str("# instantWM configuration\n");
    out.push_str("#\n");
    out.push_str(
        "# This file is optional. instantWM uses sensible defaults when no config exists.\n",
    );
    out.push_str("# Uncomment and modify any section below to override defaults.\n");
    out.push_str("#\n");
    out.push_str("# Config changes are applied on reload (instantwmctl reload).\n");
    out.push_str("#\n");
    out.push_str(
        "# Use `instantwm --print-config` to see the full default config with all values.\n",
    );
    out.push_str("# Use `instantwm --list-actions` to see valid action names for keybinds.\n");
    out.push_str("#\n\n");

    for line in full.lines() {
        if line.trim().is_empty() {
            out.push('\n');
        } else if line.starts_with('[') {
            out.push_str("# ");
            out.push_str(line);
            out.push('\n');
        } else {
            out.push_str("# ");
            out.push_str(line);
            out.push('\n');
        }
    }

    out
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

#[cfg(test)]
mod theme_tests {
    use super::*;

    fn parse(source: &str) -> ThemeConfig {
        let value = toml::from_str(source).unwrap();
        resolve_theme_colors(value)
            .unwrap()
            .try_into::<ThemeConfig>()
            .unwrap()
    }

    #[test]
    fn built_in_theme_is_used_as_color_base() {
        let config = parse(r#"theme = "nord""#);
        assert_eq!(config.theme, ColorTheme::Nord);
        assert_eq!(config.colors.status.bg, "#2e3440".parse().unwrap());
        assert_eq!(config.colors.border.tile_focus, "#81a1c1".parse().unwrap());
    }

    #[test]
    fn individual_colors_override_the_selected_theme() {
        let config = parse(
            r##"
            theme = "catppuccin-latte"
            [colors.status]
            bg = "#123456"
            "##,
        );
        assert_eq!(config.colors.status.bg, "#123456".parse().unwrap());
        assert_eq!(config.colors.status.fg, "#4c4f69".parse().unwrap());
        assert_eq!(config.colors.border.tile_focus, "#1e66f5".parse().unwrap());
    }

    #[test]
    fn every_documented_theme_name_deserializes() {
        for name in [
            "instantos",
            "catppuccin-latte",
            "catppuccin-frappe",
            "catppuccin-macchiato",
            "catppuccin-mocha",
            "nord",
            "gruvbox",
        ] {
            parse(&format!("theme = {name:?}"));
        }
    }

    #[test]
    fn display_names_match_serde_names_for_every_variant() {
        for theme in ColorTheme::ALL {
            assert_eq!(theme.to_string().parse(), Ok(*theme));
        }
        assert!("not-a-theme".parse::<ColorTheme>().is_err());
    }

    #[test]
    fn invalid_theme_falls_back_without_discarding_other_settings() {
        let config = parse(
            r#"
            theme = "does-not-exist"

            [layout]
            inner_gap = 7
            "#,
        );
        // Bad theme name is a warning, not a hard error: it falls back to the
        // default theme…
        assert_eq!(config.theme, ColorTheme::Instantos);
        // …and the rest of the config still loads.
        assert_eq!(config.layout.inner_gap, 7);
    }
}
