use crate::config::keybind_config::KeybindSpec;
use crate::core_state::FontConfig;
use crate::types::{
    BorderColorConfig, CloseButtonColorConfigs, KeyboardLayout, Rule, StatusColorConfig,
    TagColorConfigs, WindowColorConfigs,
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

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct UserConfig {
    /// Built-in colour theme used as the base for `[colors]` overrides.
    pub theme: ColorTheme,
    pub includes: Vec<IncludeConfig>,
    pub fonts: FontConfig,
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
    /// Animation timing configuration.
    pub animations: AnimationConfig,
    /// Status bar visibility and geometry.
    pub bar: BarConfig,
    /// Raise a floating window when its client area is left-clicked.
    ///
    /// Disabled by default so focus-follows-mouse and click-to-focus do not
    /// disturb the explicit floating-window stack.
    pub raise_floating_on_click: bool,
    /// Window rules.
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Commands to execute once at startup (like sway `exec` / Hyprland `exec-once`).
    #[serde(default)]
    pub exec_once: Vec<String>,
    /// Commands to execute at startup and on every config reload (like sway `exec_always`).
    #[serde(default)]
    pub exec: Vec<String>,
}

/// Status bar settings shared by the user schema and effective configuration.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct BarConfig {
    pub show: bool,
    /// Show the bottom gesture strip (plain background, no contents).
    pub show_bottom: bool,
    /// Bar height in logical pixels. `0` derives it from font metrics.
    pub height: i32,
    /// Width of the start-menu hit target in logical pixels.
    pub startmenu_size: i32,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            show: true,
            show_bottom: false,
            height: 0,
            startmenu_size: 30,
        }
    }
}

impl BarConfig {
    pub fn validated(self) -> Result<Self, String> {
        if self.height < 0 {
            return Err(format!(
                "bar.height must be non-negative, got {}",
                self.height
            ));
        }
        if self.startmenu_size < 0 {
            return Err(format!(
                "bar.startmenu_size must be non-negative, got {}",
                self.startmenu_size
            ));
        }
        Ok(self)
    }
}

/// Validated animation speed multiplier.
///
/// `1.0` is the neutral/default speed, `0.5` doubles durations, and `2.0`
/// halves them. Keeping the invariant in this type means animation code never
/// has to handle zero, non-finite, or absurd duration divisors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationSpeed(f64);

impl AnimationSpeed {
    pub const MIN: f64 = 0.01;
    pub const MAX: f64 = 100.0;
    pub const DEFAULT: f64 = 1.0;

    pub const fn get(self) -> f64 {
        self.0
    }

    pub fn scale_duration(self, duration: std::time::Duration) -> std::time::Duration {
        if duration.is_zero() {
            return duration;
        }
        std::time::Duration::from_secs_f64(duration.as_secs_f64() / self.0)
    }
}

impl Default for AnimationSpeed {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

impl TryFrom<f64> for AnimationSpeed {
    type Error = String;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        if !value.is_finite() || !(Self::MIN..=Self::MAX).contains(&value) {
            return Err(format!(
                "animation speed must be finite and between {} and {}",
                Self::MIN,
                Self::MAX
            ));
        }
        Ok(Self(value))
    }
}

impl Serialize for AnimationSpeed {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_f64(self.0)
    }
}

impl<'de> Deserialize<'de> for AnimationSpeed {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = f64::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

/// Animation timing configuration.
///
/// ```toml
/// [animations]
/// # 0.5 = half speed; 2.0 = twice as fast
/// speed = 1.0
/// ```
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Serialize)]
#[serde(default)]
#[derive(Default)]
pub struct AnimationConfig {
    pub speed: AnimationSpeed,
}

impl AnimationConfig {
    pub fn scale_duration(self, duration: std::time::Duration) -> std::time::Duration {
        self.speed.scale_duration(duration)
    }
}

/// A built-in base colour theme. Names use kebab-case in TOML.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Decode, Encode)]
#[serde(rename_all = "kebab-case")]
pub enum ColorTheme {
    Classic,
    CatppuccinLatte,
    CatppuccinFrappe,
    CatppuccinMacchiato,
    #[default]
    CatppuccinMocha,
    Nord,
    Gruvbox,
}

impl ColorTheme {
    /// All built-in themes, in the order shown by `instantwmctl theme --list`.
    pub const ALL: &[ColorTheme] = &[
        ColorTheme::Classic,
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
            Self::Classic => "classic",
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
/// maximized_gaps = false
/// keyboard_resize_step = 0.05
/// minimum_weight = 0.15
/// pointer_edge_fraction = 0.34
/// new_window_placement = "auto-resize"
/// ```
#[derive(Debug, Deserialize, Clone, Copy, Serialize)]
#[serde(default)]
pub struct LayoutConfig {
    /// Gap between tiled windows in logical pixels.
    pub inner_gap: i32,
    /// Gap between tiled windows and the monitor work area edge in logical pixels.
    pub outer_gap: i32,
    /// Disable gaps when a tiling layout has one or fewer tiled windows.
    pub smart_gaps: bool,
    /// Apply configured gaps to maximized-stack presentation.
    pub maximized_gaps: bool,
    /// Fraction of an axis changed by one manual-tree keyboard resize.
    pub keyboard_resize_step: f64,
    /// Preferred minimum weight for a child in a manual axis run.
    pub minimum_weight: f64,
    /// Fraction of a target window occupied by pointer edge-placement bands.
    pub pointer_edge_fraction: f64,
    /// How newly tiled windows are inserted into the persistent layout tree.
    pub new_window_placement: NewWindowPlacement,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            inner_gap: 0,
            outer_gap: 0,
            smart_gaps: false,
            maximized_gaps: false,
            keyboard_resize_step: 0.05,
            minimum_weight: 0.15,
            pointer_edge_fraction: 0.34,
            new_window_placement: NewWindowPlacement::default(),
        }
    }
}

impl LayoutConfig {
    /// Validate the invariants expected by layout code without silently
    /// changing user input.
    pub fn validated(self) -> Result<Self, String> {
        fn validate_fraction(
            field: &str,
            value: f64,
            minimum: f64,
            maximum: f64,
        ) -> Result<(), String> {
            if !value.is_finite() || !(minimum..=maximum).contains(&value) {
                return Err(format!(
                    "layout.{field} must be finite and between {minimum} and {maximum}, got {value}"
                ));
            }
            Ok(())
        }

        if self.inner_gap < 0 {
            return Err(format!(
                "layout.inner_gap must be non-negative, got {}",
                self.inner_gap
            ));
        }
        if self.outer_gap < 0 {
            return Err(format!(
                "layout.outer_gap must be non-negative, got {}",
                self.outer_gap
            ));
        }
        validate_fraction(
            "keyboard_resize_step",
            self.keyboard_resize_step,
            0.001,
            0.5,
        )?;
        validate_fraction("minimum_weight", self.minimum_weight, 0.001, 0.49)?;
        validate_fraction(
            "pointer_edge_fraction",
            self.pointer_edge_fraction,
            0.05,
            0.49,
        )?;
        Ok(self)
    }
}

/// Policy for inserting a window which is not yet represented in a tag's
/// persistent tiling tree.
#[derive(Debug, Default, Deserialize, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NewWindowPlacement {
    /// Split the best existing leaf without deliberately rebalancing unrelated
    /// branches.
    Auto,
    /// Use automatic placement, but give a cramped newcomer a larger root-level
    /// region and proportionally resize the existing tree.
    #[default]
    AutoResize,
    /// Make the first newcomer the leading half of a new vertical root split.
    /// Consecutive untouched force insertions adapt that generated region into
    /// balanced rows or columns; a manual tree edit starts a new sequence.
    Force,
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

impl From<bool> for ToggleSetting {
    fn from(enabled: bool) -> Self {
        if enabled {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
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
    /// Output receiving absolute events from this input device.
    ///
    /// Use a connector name such as `eDP-1`. `*` maps the device across the
    /// complete active output layout.
    pub map_to_output: Option<String>,
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
            map_to_output: None,
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
        writeln!(f, "left_handed: {:?}", self.left_handed)?;
        write!(f, "map_to_output: {:?}", self.map_to_output)
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
/// swapescape = true
/// ```
#[derive(Debug, Deserialize, Clone, Serialize, Default)]
#[serde(default)]
pub struct KeyboardConfig {
    /// XKB layout configurations.
    #[serde(default)]
    pub layouts: Vec<KeyboardLayout>,
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

pub fn load_config_file() -> Result<UserConfig, String> {
    let path = match dirs::config_dir() {
        Some(dir) => dir.join("instantwm").join("config.toml"),
        None => return Ok(UserConfig::default()),
    };

    if !path.exists() {
        return Ok(UserConfig::default());
    }

    let mut visited = HashSet::new();
    let merged = load_and_merge_config(&path, &mut visited)?;
    resolve_theme_colors(merged)?
        .try_into::<UserConfig>()
        .map_err(|error| format!("config parse error in {}: {error}", path.display()))
}

fn resolve_theme_colors(mut config: toml::Value) -> Result<toml::Value, String> {
    let theme = match config.get("theme").cloned() {
        None => ColorTheme::default(),
        Some(value) => match value.clone().try_into::<ColorTheme>() {
            Ok(theme) => theme,
            Err(_) => {
                eprintln!("instantwm: unknown theme {value}, falling back to the default theme");
                // Drop the bad key so UserConfig deserialisation succeeds;
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
    let config = UserConfig::default();
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
        } else {
            out.push_str("# ");
            out.push_str(line);
            out.push('\n');
        }
    }

    out
}

fn load_and_merge_config(
    path: &Path,
    visited: &mut HashSet<PathBuf>,
) -> Result<toml::Value, String> {
    let canonical_path = path.canonicalize().map_err(|error| {
        format!(
            "could not canonicalize config path {}: {error}",
            path.display()
        )
    })?;

    if visited.contains(&canonical_path) {
        return Err(format!(
            "circular config include detected at {}",
            canonical_path.display()
        ));
    }
    visited.insert(canonical_path.clone());

    let contents = fs::read_to_string(path)
        .map_err(|error| format!("could not read config file {}: {error}", path.display()))?;

    let value: toml::Value = toml::from_str(&contents)
        .map_err(|error| format!("config parse error in {}: {error}", path.display()))?;

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
                    return Err(format!(
                        "included config file {} does not exist",
                        include_path.display()
                    ));
                }

                let included_value = load_and_merge_config(&include_path, visited)?;
                merge_toml_values(&mut merged_base, included_value);
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

    fn parse(source: &str) -> UserConfig {
        let value = toml::from_str(source).unwrap();
        resolve_theme_colors(value)
            .unwrap()
            .try_into::<UserConfig>()
            .unwrap()
    }

    #[test]
    fn layout_validation_reports_the_invalid_field() {
        for (config, field) in [
            (
                LayoutConfig {
                    inner_gap: -4,
                    ..LayoutConfig::default()
                },
                "layout.inner_gap",
            ),
            (
                LayoutConfig {
                    keyboard_resize_step: f64::INFINITY,
                    ..LayoutConfig::default()
                },
                "layout.keyboard_resize_step",
            ),
            (
                LayoutConfig {
                    minimum_weight: 0.9,
                    ..LayoutConfig::default()
                },
                "layout.minimum_weight",
            ),
            (
                LayoutConfig {
                    pointer_edge_fraction: 0.0,
                    ..LayoutConfig::default()
                },
                "layout.pointer_edge_fraction",
            ),
        ] {
            let error = config.validated().unwrap_err();
            assert!(error.contains(field), "{error}");
        }

        assert!(LayoutConfig::default().validated().is_ok());
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
    fn floating_click_raise_is_an_explicit_opt_in() {
        assert!(!parse("").raise_floating_on_click);
        assert!(parse("raise_floating_on_click = true").raise_floating_on_click);
    }

    #[test]
    fn bottom_bar_is_off_by_default_and_explicitly_opt_in() {
        // Disabled by default — the bottom bar is a gesture surface the user
        // must enable via config, IPC toggle, or `Super+Shift+B`.
        assert!(!parse("").bar.show_bottom);
        assert!(parse("[bar]\nshow_bottom = true").bar.show_bottom);
    }

    #[test]
    fn font_roles_are_explicit_and_independently_sized() {
        let config = parse(
            r#"
            [fonts]
            text_family = "Iosevka"
            icon_size = 18.0
            "#,
        );

        assert_eq!(config.fonts.text_family, "Iosevka");
        assert_eq!(config.fonts.text_size, 12.0);
        assert_eq!(config.fonts.icon_family, "Symbols Nerd Font");
        assert_eq!(config.fonts.icon_size, 18.0);
    }

    #[test]
    fn legacy_ordered_font_array_is_rejected() {
        let value = toml::from_str::<toml::Value>(
            r#"fonts = ["Inter:size=12", "Fira Code Nerd Font:size=12"]"#,
        )
        .unwrap();
        assert!(value.try_into::<UserConfig>().is_err());
    }

    #[test]
    fn bar_settings_have_one_user_visible_source() {
        let bar = parse("[bar]\nshow = false\nheight = 32\nstartmenu_size = 44")
            .bar
            .validated()
            .unwrap();

        assert!(!bar.show);
        assert_eq!(bar.height, 32);
        assert_eq!(bar.startmenu_size, 44);
        assert!(BarConfig { height: -1, ..bar }.validated().is_err());
    }

    #[test]
    fn animation_speed_defaults_to_the_neutral_multiplier() {
        let default = parse("").animations;
        let neutral = parse("[animations]\nspeed = 1.0").animations;
        let slow = parse("[animations]\nspeed = 0.25").animations;
        let fast = parse("[animations]\nspeed = 2.0").animations;
        let base = std::time::Duration::from_millis(100);

        assert_eq!(default.speed.get(), AnimationSpeed::DEFAULT);
        assert_eq!(default.scale_duration(base), base);
        assert_eq!(neutral.scale_duration(base), base);
        assert_eq!(
            slow.scale_duration(base),
            std::time::Duration::from_millis(400)
        );
        assert_eq!(
            fast.scale_duration(base),
            std::time::Duration::from_millis(50)
        );
    }

    #[test]
    fn invalid_animation_speeds_are_rejected() {
        for speed in ["0.0", "-1.0", "0.001", "101.0", "nan", "inf"] {
            let source = format!("[animations]\nspeed = {speed}");
            let value = toml::from_str(&source).unwrap();
            let resolved = resolve_theme_colors(value).unwrap();
            assert!(
                resolved.try_into::<UserConfig>().is_err(),
                "accepted {speed}"
            );
        }
    }

    #[test]
    fn new_window_placement_defaults_to_auto_resize_and_accepts_all_policies() {
        assert_eq!(
            parse("").layout.new_window_placement,
            NewWindowPlacement::AutoResize
        );
        for (name, expected) in [
            ("auto", NewWindowPlacement::Auto),
            ("auto-resize", NewWindowPlacement::AutoResize),
            ("force", NewWindowPlacement::Force),
        ] {
            let config = parse(&format!("[layout]\nnew_window_placement = {name:?}"));
            assert_eq!(config.layout.new_window_placement, expected);
        }
    }

    #[test]
    fn every_documented_theme_name_deserializes() {
        for name in [
            "classic",
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
        assert_eq!(config.theme, ColorTheme::default());
        // …and the rest of the config still loads.
        assert_eq!(config.layout.inner_gap, 7);
    }
}
