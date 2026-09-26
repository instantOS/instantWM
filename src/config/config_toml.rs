use super::appearance::ColorConfig;
use crate::config::keybind_config::KeybindSpec;
use crate::core_state::{FontConfig, SystrayConfig, WindowConfig};
use crate::types::{KeyboardLayout, Rule};
use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// One entry of a config file's `includes` list: a file whose contents are
/// inlined as if they had been written at that point.
///
/// This is a *loader directive*, not a configuration value: [`load_config_file`]
/// consumes it while building the merged TOML tree, so it never reaches
/// [`UserConfig`] and is not part of the effective config. It lives here
/// because this module owns the config schema, and it is the only description of
/// the `includes` syntax — the loader deserialises through it rather than
/// reaching into the raw table, so the two cannot drift apart.
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IncludeSpec {
    /// Path of the file to inline. A leading `~/` is the user's home directory,
    /// and a relative path resolves against the directory of the *including*
    /// file. See [`IncludeSpec::resolve`].
    pub file: PathBuf,
}

impl IncludeSpec {
    /// Resolve this entry against the file that included it.
    ///
    /// A leading `~` or `~/` is the user's home directory, so a config can name
    /// a file in `$HOME` without the user having to spell out their home path.
    /// Otherwise an absolute path is taken as written, and a relative path
    /// resolves against `including_file`'s directory — not the process working
    /// directory — so a split-up config resolves the same way no matter which
    /// file pulls it in, and `instantwm` started from anywhere still finds it.
    fn resolve(&self, including_file: &Path) -> Result<PathBuf, String> {
        // The tilde is matched on raw text: `Path` compares whole components, so
        // it cannot tell `~/colors.toml` from the `~user/colors.toml` form.
        if let Some(text) = self.file.to_str().filter(|text| text.starts_with('~')) {
            return match text {
                "~" => home_dir(),
                _ => match text.strip_prefix("~/") {
                    Some(rest) => Ok(home_dir()?.join(rest)),
                    // `~user` names another account, which needs a passwd lookup
                    // this does not do. Say so rather than resolving a literal
                    // directory that happens to be called `~user`.
                    None => Err(format!(
                        "unsupported include path '{text}': \
                         use '~/' for your home directory, or an absolute path"
                    )),
                },
            };
        }

        if self.file.is_absolute() {
            return Ok(self.file.clone());
        }

        Ok(including_file
            .parent()
            .unwrap_or(Path::new("."))
            .join(&self.file))
    }
}

/// The user's home directory, for `~` in an include path.
fn home_dir() -> Result<PathBuf, String> {
    dirs::home_dir()
        .ok_or_else(|| "cannot expand '~' in an include path: no home directory is set".to_string())
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

/// The user's configuration, as a single value.
///
/// A value of this type always describes an *already-merged* config: the
/// `includes` directive is resolved by [`load_config_file`] before this struct
/// is built, which is why `includes` is not a field here. See [`IncludeSpec`].
#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct UserConfig {
    /// Built-in colour theme used as the base for `[colors]` overrides.
    pub theme: ColorTheme,
    pub fonts: FontConfig,
    pub colors: ColorConfig,
    /// User-defined keybinds (override/extend defaults).
    pub keybinds: Vec<KeybindSpec>,
    /// User-defined desktop keybinds (override/extend defaults).
    pub desktop_keybinds: Vec<KeybindSpec>,
    /// Keyboard layout configuration.
    pub keyboard: KeyboardConfig,
    /// Input configuration (mouse, touchpad).
    //BOZO: is string the right type here? Or something more narrow?
    pub input: HashMap<String, InputConfig>,
    /// Monitor configuration.
    pub monitors: HashMap<String, MonitorConfig>,
    /// Background command to execute for reading status bar text, typically `i3status-rs`
    pub status_command: Option<String>,
    /// User-defined modes (sway-like modes).
    pub modes: HashMap<String, ModeSpec>,
    /// Cursor configuration (Wayland only).
    pub cursor: CursorConfig,
    /// Focus navigation settings.
    pub focus: FocusConfig,
    /// Tag display settings.
    pub tags: TagsConfig,
    /// Layout geometry configuration.
    pub layout: LayoutConfig,
    /// Animation timing configuration.
    pub animations: AnimationConfig,
    /// Status bar visibility and geometry.
    pub bar: BarConfig,
    /// System tray settings.
    pub systray: SystrayConfig,
    /// Window behaviour: border width, snapping, size hints and decoration
    /// hints.
    pub window: WindowConfig,
    /// Raise a floating window when its client area is left-clicked.
    ///
    /// Legacy top-level spelling of `window.raise_floating_on_click`; either
    /// location enables it.
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
    /// Actions to run in response to events such as monitor hotplug.
    #[serde(default)]
    pub hooks: Vec<crate::config::hooks::HookSpec>,
}

/// Status bar settings shared by the user schema and effective configuration.
///
/// Every tag-display setting here is a *default for all outputs*; an output
/// may override it in its `[monitors.<name>]` entry (or `[monitors."*"]` for
/// all of them). See [`crate::bar::policy::TagBarPolicy`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BarConfig {
    pub show: bool,
    /// Show the bottom gesture strip (plain background, no contents).
    pub show_bottom: bool,
    /// Number of leading tags considered for the bar, 1..=20. Occupied and
    /// selected tags beyond this baseline are also shown. When the baseline
    /// is full, the first empty tag to its right is shown too.
    /// `[monitors.<name>].tag_slots` overrides this per output.
    pub tag_slots: u32,
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
            tag_slots: crate::types::tag::DEFAULT_TAG_SLOTS,
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
        validate_tag_slots(self.tag_slots, "bar.tag_slots")?;
        Ok(self)
    }
}

/// Shared bounds for a tag-cell count: at least one cell, at most one per tag.
pub(crate) fn validate_tag_slots(slots: u32, field: &str) -> Result<(), String> {
    if slots < 1 || slots as usize > crate::types::SCRATCHPAD_TAG {
        return Err(format!(
            "{field} must be between 1 and {}, got {slots}",
            crate::types::SCRATCHPAD_TAG
        ));
    }
    Ok(())
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
/// enabled = true
/// # 0.5 = half speed; 2.0 = twice as fast
/// speed = 1.0
/// ```
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Serialize)]
#[serde(default)]
pub struct AnimationConfig {
    /// Master switch for window animations. `instantwmctl config toggle
    /// animations.enabled` flips this at runtime; `instantwmctl reload`
    /// restores the configured value.
    pub enabled: bool,
    pub speed: AnimationSpeed,
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            speed: AnimationSpeed::default(),
        }
    }
}

impl AnimationConfig {
    pub fn scale_duration(self, duration: std::time::Duration) -> std::time::Duration {
        self.speed.scale_duration(duration)
    }
}

/// A built-in base colour theme. Names use kebab-case in TOML.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Deserialize,
    Serialize,
    Decode,
    Encode,
    clap::ValueEnum,
)]
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
    pub fn name(self) -> String {
        clap::ValueEnum::to_possible_value(&self)
            .expect("every theme has a name")
            .get_name()
            .to_string()
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
            smart_gaps: true,
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

/// What horizontal focus navigation does once it runs out of windows.
///
/// This only ever decides the *fallthrough*: `focus_left` / `focus_right`
/// always try to move to a neighbouring window first, and only ask this
/// policy what to do when there is none left in that direction on the
/// current tag.
#[derive(Debug, Default, Deserialize, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HorizontalEdge {
    /// Continue into the adjacent tag, the traditional window manager
    /// behaviour. Reaching the first or last tag simply stops.
    #[default]
    Overflow,
    /// Cycle focus to the opposite end of the current tag. The tag never
    /// changes, so a key press at the spatial edge stays on this workspace.
    Wrap,
    /// Consume the key press without moving focus or tags.
    None,
}

/// What vertical focus navigation does once it runs out of windows.
///
/// Deliberately a separate, smaller value set than [`HorizontalEdge`]:
/// `overflow` is the "carry on into the adjacent workspace" policy, and tags
/// only vary along the horizontal axis, so there is nothing for it to select
/// on this one. Only wrapping within the tag and stopping are available.
#[derive(Debug, Default, Deserialize, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VerticalEdge {
    /// Jump focus to the opposite edge of the tag — no window is above, so
    /// take the bottom-most one, and vice versa. This is the default and the
    /// exact counterpart of [`HorizontalEdge::Wrap`].
    ///
    /// Maximized presentation is the one exception: every window occupies the
    /// same region there, so there is no top or bottom to wrap across and the
    /// cycle falls back to bar order instead.
    #[default]
    Wrap,
    /// Consume the key press without moving focus.
    None,
}

/// Focus navigation configuration from the TOML `[focus]` section.
///
/// Both fields only ever decide the *fallthrough*: directional focus always
/// moves to a neighbour first and asks the policy what to do only once there
/// is none left in that direction. Their value sets differ because only the
/// horizontal axis has a neighbouring workspace to overflow into.
#[derive(Debug, Default, Deserialize, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FocusConfig {
    /// What `focus_left` / `focus_right` do once focus has run out of
    /// windows in that direction on the current tag. The default, `overflow`,
    /// is the traditional compromise between a tiling window manager and
    /// a traditional one: walk the windows, then step to the next tag.
    pub horizontal_edge: HorizontalEdge,
    /// What `focus_up` / `focus_down` do once focus has run out of windows
    /// above or below on the current tag. The default, `wrap`, jumps to the
    /// opposite edge; `none` stops at it. There is no `overflow` because tags
    /// do not stack vertically.
    pub vertical_edge: VerticalEdge,
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
///
/// Doubles as the `instantwmctl monitor set` patch: `None` fields keep the
/// current value.
#[derive(Debug, Deserialize, Clone, Serialize, Default, Encode, Decode, clap::Args)]
#[serde(default, deny_unknown_fields)]
pub struct MonitorConfig {
    /// Resolution in "WIDTHxHEIGHT" format (e.g., "1920x1080").
    #[arg(long, short = 'r')]
    pub resolution: Option<String>,
    /// Refresh rate in Hz (e.g., 60.0).
    #[arg(long, short = 'f')]
    pub refresh_rate: Option<f32>,
    /// Position in "X,Y" format (e.g., "1920,0") or relative (e.g., "left-of:DP-1").
    #[arg(long, short = 'p')]
    pub position: Option<String>,
    /// Scale factor (e.g., 1.0, 2.0).
    #[arg(long, short = 's')]
    pub scale: Option<f32>,
    /// Output rotation and reflection.
    #[arg(long, short = 't')]
    pub transform: Option<Transform>,
    /// Whether the monitor is enabled.
    #[arg(long)]
    pub enable: Option<bool>,
    /// Variable refresh rate policy for this output.
    #[arg(long)]
    pub vrr: Option<VrrMode>,
    /// Name of the source output this output mirrors ("clone"). The mirror head
    /// shows the source's region of the desktop, and both heads form a single
    /// logical monitor, so the mirror's own `position` and `scale` are ignored.
    /// On Wayland its `resolution`, `refresh_rate` and `transform` still select
    /// how the head scans out, and the source's content is fitted to it; X11
    /// cannot scale and derives the mirror's mode from the source. A mirror
    /// whose source is disconnected or disabled is an ordinary output until
    /// the source returns. Empty/`none` clears the mirror.
    #[arg(long, value_name = "OUTPUT|none")]
    pub mirror: Option<String>,
    /// How the mirror fits its source's content when the two framebuffers'
    /// aspect ratios differ. Only meaningful together with `mirror`; ignored
    /// (and cleared at apply time) on a non-mirror output. Wayland only.
    #[arg(long)]
    pub mirror_fit: Option<MirrorFit>,
    /// Leading tag baseline on this output, overriding
    /// `bar.tag_slots`. Omitted means "inherit".
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=crate::types::SCRATCHPAD_TAG as i64))]
    pub tag_slots: Option<u32>,
}

impl MonitorConfig {
    /// Reject values the WM cannot present. The hardware fields are
    /// sanitized elsewhere (mirrors) or clamped by the backends; this only
    /// covers the display-policy fields, which must fail loudly. `key` is
    /// the config entry name (`*` for the wildcard) and only labels errors.
    pub fn validated(&self, key: &str) -> Result<(), String> {
        if let Some(slots) = self.tag_slots {
            validate_tag_slots(slots, &format!("monitors.{key}.tag_slots"))?;
        }
        Ok(())
    }
}

/// Output transform, named as in config and on the command line.
#[derive(
    Debug, Deserialize, Clone, Copy, PartialEq, Eq, Serialize, Encode, Decode, clap::ValueEnum,
)]
pub enum Transform {
    #[serde(rename = "normal")]
    Normal,
    #[serde(rename = "90")]
    #[value(name = "90")]
    Rotate90,
    #[serde(rename = "180")]
    #[value(name = "180")]
    Rotate180,
    #[serde(rename = "270")]
    #[value(name = "270")]
    Rotate270,
    #[serde(rename = "flipped")]
    Flipped,
    #[serde(rename = "flipped-90")]
    #[value(name = "flipped-90")]
    Flipped90,
    #[serde(rename = "flipped-180")]
    #[value(name = "flipped-180")]
    Flipped180,
    #[serde(rename = "flipped-270")]
    #[value(name = "flipped-270")]
    Flipped270,
}

impl From<Transform> for crate::backend::output::OutputTransform {
    fn from(transform: Transform) -> Self {
        match transform {
            Transform::Normal => Self::Normal,
            Transform::Rotate90 => Self::Rotate90,
            Transform::Rotate180 => Self::Rotate180,
            Transform::Rotate270 => Self::Rotate270,
            Transform::Flipped => Self::Flipped,
            Transform::Flipped90 => Self::Flipped90,
            Transform::Flipped180 => Self::Flipped180,
            Transform::Flipped270 => Self::Flipped270,
        }
    }
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
    #[default]
    Off,
    Auto,
    On,
}

/// How a mirrored output fits its source's content when the aspect ratios
/// differ: letterbox bars ([`MirrorFit::Contain`]) or crop to fill
/// ([`MirrorFit::Cover`]).
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
pub enum MirrorFit {
    /// Scale the source down until it fits entirely inside the mirror's
    /// framebuffer, centering it; the leftover axes show background bars.
    #[default]
    Contain,
    /// Scale the source up until it covers the mirror's framebuffer
    /// completely, centering it; the overflowing edges are cropped.
    Cover,
}

/// Toggle setting for boolean-like input options (tap, natural_scroll).
///
/// Serde accepts the CLI's `on`/`off` aliases so `config set input.*.tap on`
/// and the TOML schema speak the same grammar as `instantwmctl mouse tap`.
#[derive(
    Debug, Deserialize, Clone, Copy, PartialEq, Eq, Serialize, Encode, Decode, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum ToggleSetting {
    #[value(alias = "on")]
    #[serde(alias = "on")]
    Enabled,
    #[value(alias = "off")]
    #[serde(alias = "off")]
    Disabled,
}

/// Acceleration profile for pointer devices.
#[derive(
    Debug, Deserialize, Clone, Copy, PartialEq, Eq, Serialize, Encode, Decode, clap::ValueEnum,
)]
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

/// Tag labels, icons, and bar display mode.
///
/// `count` is the number of ordinary tags (maximum 20; the scratchpad bit is
/// reserved). `names` and `icons` are optional positional labels. Missing
/// names use their numbered tag and missing icons are empty.
///
/// ```toml
/// [tags]
/// count = 20
/// names = ["1", "2", "web", "mail"]
/// # Nerd-font glyphs, shown instead of the names while show_icons is on.
/// icons = ["", "", "", ""]
/// show_icons = false
/// ```
///
/// `count` defines the tag set; `names` and `icons` label it. All three take
/// effect on startup and `reload` only; `instantwmctl config set` rejects them. Rename tags
/// for the session with `instantwmctl tag name <label>`, and drop the
/// session renames with `instantwmctl tag reset`.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TagsConfig {
    /// Number of ordinary tags. The scratchpad uses a separate reserved bit.
    /// Reload rejects a reduction while a window or active view uses a
    /// removed tag.
    pub count: usize,
    /// Optional label per tag, index 0 = tag 1. Missing labels are numbered.
    pub names: Vec<String>,
    /// Optional icon per tag (see the type docs for the positional rule).
    /// Defaults to no icons at all.
    pub icons: Vec<String>,
    /// Show icons instead of names in the tag bar.
    /// `instantwmctl config toggle tags.show_icons` (or the `config_toggle`
    /// action) flips this for the session; `reload` restores the configured
    /// value.
    pub show_icons: bool,
}

impl Default for TagsConfig {
    fn default() -> Self {
        Self {
            count: crate::types::SCRATCHPAD_TAG,
            icons: Vec::new(),
            names: Vec::new(),
            show_icons: false,
        }
    }
}

impl TagsConfig {
    /// Reject tag sets the WM cannot present.
    pub fn validated(self) -> Result<Self, String> {
        if !(1..=crate::types::SCRATCHPAD_TAG).contains(&self.count) {
            return Err(format!(
                "tags.count must be between 1 and {}, got {}",
                crate::types::SCRATCHPAD_TAG,
                self.count
            ));
        }
        if self.names.len() > self.count {
            return Err(format!(
                "tags.names has {} entries but tags.count is {}",
                self.names.len(),
                self.count
            ));
        }
        for (index, name) in self.names.iter().enumerate() {
            if name.is_empty() {
                return Err(format!("tags.names[{index}] must not be empty"));
            }
            if name.len() > crate::types::tag::MAX_TAG_NAME_BYTES {
                return Err(format!(
                    "tags.names[{index}] is {} bytes; at most {} are allowed",
                    name.len(),
                    crate::types::tag::MAX_TAG_NAME_BYTES
                ));
            }
        }
        if self.icons.len() > self.count {
            return Err(format!(
                "tags.icons has {} entries but tags.count is {}",
                self.icons.len(),
                self.count
            ));
        }
        Ok(self)
    }

    /// The tag template the model is initialised from: `names` paired with
    /// `icons`, padding the icon list with empty entries.
    pub fn tag_template(&self) -> Vec<crate::types::Tag> {
        (0..self.count)
            .map(|index| crate::types::Tag {
                name: self
                    .names
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| (index + 1).to_string()),
                icon: self.icons.get(index).cloned().unwrap_or_default(),
            })
            .collect()
    }
}

/// Read the user's config, resolving `includes` and theme colours first.
///
/// This is the boundary between the *file on disk* — which may be split across
/// several files and may select a theme — and the single merged [`UserConfig`]
/// value the rest of the window manager sees.
pub fn load_config_file() -> Result<UserConfig, String> {
    let path = match dirs::config_dir() {
        Some(dir) => dir.join("instantwm").join("config.toml"),
        None => return Ok(UserConfig::default()),
    };

    if !path.exists() {
        return Ok(UserConfig::default());
    }

    let mut state = IncludeState::default();
    let merged = load_and_merge_config(&path, &mut state)?;
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
    let mut config = UserConfig::default();
    // The stock tag set has no icons; show the field as an empty list
    // instead of one empty string per tag.
    config.tags.icons.clear();
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
        "# A config can be split across files. Each `includes` entry names a file to inline as if it\n\
         # were written here. The including file wins on conflicts, `~/` is your home directory, and\n\
         # any other relative path resolves against the file that includes it:\n\
         #\n\
         #     includes = [{ file = \"colors.toml\" }, { file = \"~/dotfiles/keys.toml\" }]\n\
         #\n",
    );
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

/// Include-recursion bookkeeping for [`load_and_merge_config`].
///
/// The two sets answer different questions and both are needed. `stack` holds
/// the files currently being loaded, so a file that includes one of its own
/// ancestors is a cycle. `merged` holds the files that have already been
/// inlined, so a file reachable through two different include chains is applied
/// once instead of twice. Conflating them — as a single ever-seen set does —
/// turns every diamond include into a spurious "circular include" error.
#[derive(Default)]
struct IncludeState {
    /// Canonical paths of the files currently being loaded, outermost first.
    stack: Vec<PathBuf>,
    /// Canonical paths of the files already merged into the result. Disjoint
    /// from `stack` by construction: a path is only added once it is off the
    /// stack and fully merged.
    merged: HashSet<PathBuf>,
}

/// Read one config file and return it with its `includes` inlined.
///
/// Included files form the base and this file is merged over them, so a value
/// written in the including file always wins. Later includes are merged over
/// earlier ones, and arrays accumulate rather than replace, which is what makes
/// splitting a config across files compose the way it reads.
fn load_and_merge_config(path: &Path, state: &mut IncludeState) -> Result<toml::Value, String> {
    let canonical_path = path.canonicalize().map_err(|error| {
        format!(
            "could not canonicalize config path {}: {error}",
            path.display()
        )
    })?;

    if state.stack.contains(&canonical_path) {
        return Err(format!(
            "circular config include: {}",
            describe_include_chain(state, &canonical_path)
        ));
    }

    // Already inlined by an earlier include chain. Merging it again would
    // duplicate every array it contributes, and its scalar values cannot
    // disagree with themselves, so there is nothing left to contribute.
    if state.merged.contains(&canonical_path) {
        return Ok(toml::Value::Table(toml::Table::new()));
    }

    let contents = fs::read_to_string(path)
        .map_err(|error| format!("could not read config file {}: {error}", path.display()))?;

    let mut value: toml::Value = toml::from_str(&contents)
        .map_err(|error| format!("config parse error in {}: {error}", path.display()))?;

    // Taken, not copied: `includes` is consumed here, which is what keeps it
    // out of the merged tree and therefore out of `UserConfig`.
    let specs = take_include_specs(&mut value, path)?;

    let mut merged_base = toml::Value::Table(toml::Table::new());

    state.stack.push(canonical_path.clone());
    for (index, spec) in specs.iter().enumerate() {
        let include_path = spec.resolve(path).map_err(|error| {
            format!(
                "config error in {}: includes[{index}]: {error}",
                path.display()
            )
        })?;

        if !include_path.exists() {
            return Err(format!(
                "config error in {}: includes[{index}] references {}, which does not exist",
                path.display(),
                include_path.display()
            ));
        }

        let included = load_and_merge_config(&include_path, state)?;
        merge_toml_values(&mut merged_base, included);
    }
    state.stack.pop();

    // Merge the current file OVER its includes.
    merge_toml_values(&mut merged_base, value);
    state.merged.insert(canonical_path);

    Ok(merged_base)
}

/// Take a file's `includes` list out of its parsed value.
///
/// Removing the key is what makes `includes` a directive rather than data: the
/// merged tree cannot carry it upwards, so no configuration value can ever
/// observe it. Unknown keys and wrong types are errors rather than skips, so a
/// mistyped include fails the load instead of quietly dropping a file.
fn take_include_specs(value: &mut toml::Value, path: &Path) -> Result<Vec<IncludeSpec>, String> {
    let Some(table) = value.as_table_mut() else {
        return Ok(Vec::new());
    };
    let Some(raw) = table.remove("includes") else {
        return Ok(Vec::new());
    };
    raw.try_into().map_err(|error| {
        format!(
            "config parse error in {}: invalid `includes`: {error}",
            path.display()
        )
    })
}

/// Render the active include chain for a cycle report, as `a -> b -> a`.
fn describe_include_chain(state: &IncludeState, repeated: &Path) -> String {
    let mut chain: Vec<String> = state
        .stack
        .iter()
        .map(|entry| entry.display().to_string())
        .collect();
    chain.push(repeated.display().to_string());
    chain.join(" -> ")
}

fn merge_toml_values(base: &mut toml::Value, over: toml::Value) {
    match (base, over) {
        (toml::Value::Table(base_table), toml::Value::Table(over_table)) => {
            for (key, value) in over_table {
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
    fn systray_menu_backend_parses_from_a_partial_section() {
        let user = parse("[systray]\nmenu_backend = \"instantmenu\"\n");
        assert_eq!(
            user.systray.menu_backend,
            crate::core_state::TrayMenuBackend::InstantMenu
        );
        // Unrelated fields keep their defaults when the section is partial.
        assert!(user.systray.show);
        assert_eq!(user.systray.spacing, 0);
    }

    #[test]
    fn systray_menu_backend_rejects_unknown_values() {
        assert!(toml::from_str::<UserConfig>("[systray]\nmenu_backend = \"popup\"\n").is_err());
    }

    #[test]
    fn input_toggles_accept_the_cli_on_off_aliases() {
        for (value, canonical, variant) in [
            (
                "\"enabled\"",
                "enabled",
                crate::config::config_toml::ToggleSetting::Enabled,
            ),
            (
                "\"disabled\"",
                "disabled",
                crate::config::config_toml::ToggleSetting::Disabled,
            ),
            (
                "\"on\"",
                "enabled",
                crate::config::config_toml::ToggleSetting::Enabled,
            ),
            (
                "\"off\"",
                "disabled",
                crate::config::config_toml::ToggleSetting::Disabled,
            ),
        ] {
            let user = parse(&format!("[input.\"type:touchpad\"]\ntap = {value}"));
            assert_eq!(user.input["type:touchpad"].tap, Some(variant));
            // Round-trip: aliases parse, but serialization stays canonical.
            let roundtrip = toml::to_string(&user).unwrap();
            assert!(
                roundtrip.contains(&format!("tap = \"{canonical}\"")),
                "expected canonical form in:\n{roundtrip}"
            );
        }
        assert!(toml::from_str::<UserConfig>("[input.x]\ntap = \"nope\"").is_err());
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
        assert_eq!(config.colors.status.background, "#2e3440".parse().unwrap());
        assert_eq!(config.colors.border.tile_focus, "#81a1c1".parse().unwrap());
    }

    #[test]
    fn individual_colors_override_the_selected_theme() {
        let config = parse(
            r##"
            theme = "catppuccin-latte"
            [colors.status]
            background = "#123456"
            "##,
        );
        assert_eq!(config.colors.status.background, "#123456".parse().unwrap());
        assert_eq!(config.colors.status.foreground, "#4c4f69".parse().unwrap());
        assert_eq!(config.colors.status.separator, "#ccd0da".parse().unwrap());
        assert_eq!(config.colors.border.tile_focus, "#1e66f5".parse().unwrap());
    }

    #[test]
    fn status_separator_color_can_be_overridden() {
        let config = parse(
            r##"
            [colors.status]
            separator = "#445566"
            "##,
        );
        assert_eq!(config.colors.status.separator, "#445566".parse().unwrap());
    }

    #[test]
    fn floating_click_raise_is_an_explicit_opt_in() {
        assert!(!parse("").raise_floating_on_click);
        assert!(parse("raise_floating_on_click = true").raise_floating_on_click);
        assert!(
            parse("[window]\nraise_floating_on_click = true")
                .window
                .raise_floating_on_click
        );
    }

    #[test]
    fn window_section_parses_and_keeps_defaults_for_missing_fields() {
        let config = parse(
            r#"
            [window]
            border_width_px = 2
            decor_hints = false
            "#,
        );
        assert_eq!(config.window.border_width_px, 2);
        assert!(!config.window.decor_hints);
        assert_eq!(config.window.snap_threshold, 32);
        assert!(config.window.resize_hints);
    }

    #[test]
    fn negative_window_values_are_rejected_by_name() {
        for (source, field) in [
            ("[window]\nborder_width_px = -1", "border_width_px"),
            ("[window]\nsnap_threshold = -1", "snap_threshold"),
        ] {
            let user: UserConfig = toml::from_str(source).unwrap();
            let error = user.window.validated().unwrap_err();
            assert!(error.contains(field), "{error}");
        }
    }

    #[test]
    fn focus_and_tag_preferences_parse_with_cli_grammar() {
        let config = parse(
            r#"
            [window]
            focus_follows_mouse = "force"
            focus_follows_float_mouse = false

            [tags]
            show_icons = true

            [bar]
            tag_slots = 5
            "#,
        );
        assert_eq!(
            config.window.focus_follows_mouse,
            crate::types::FocusFollowsMouseMode::Force
        );
        assert!(!config.window.focus_follows_float_mouse);
        assert!(config.tags.show_icons);
        assert_eq!(config.bar.tag_slots, 5);

        // Defaults for a config that omits the sections.
        let default = parse("");
        assert_eq!(
            default.window.focus_follows_mouse,
            crate::types::FocusFollowsMouseMode::Normal
        );
        assert!(default.window.focus_follows_float_mouse);
        assert!(!default.tags.show_icons);
        assert_eq!(default.bar.tag_slots, crate::types::tag::DEFAULT_TAG_SLOTS);
    }

    #[test]
    fn focus_horizontal_edge_parses_and_defaults_to_overflow() {
        for (source, expected) in [
            (
                r#"
                [focus]
                horizontal_edge = "overflow"
                "#,
                HorizontalEdge::Overflow,
            ),
            (
                r#"
                [focus]
                horizontal_edge = "wrap"
                "#,
                HorizontalEdge::Wrap,
            ),
            (
                r#"
                [focus]
                horizontal_edge = "none"
                "#,
                HorizontalEdge::None,
            ),
        ] {
            let config: UserConfig = toml::from_str(source).unwrap();
            assert_eq!(config.focus.horizontal_edge, expected);
        }

        // A config that never mentions `[focus]` keeps the historical
        // behaviour of continuing into the adjacent tag.
        let default: UserConfig = toml::from_str("").unwrap();
        assert_eq!(default.focus.horizontal_edge, HorizontalEdge::Overflow);
        assert!(toml::from_str::<UserConfig>("[focus]\nhorizontal_edge = \"bounce\"").is_err());
    }

    #[test]
    fn focus_vertical_edge_parses_and_defaults_to_wrap() {
        for (source, expected) in [
            (
                r#"
                [focus]
                vertical_edge = "wrap"
                "#,
                VerticalEdge::Wrap,
            ),
            (
                r#"
                [focus]
                vertical_edge = "none"
                "#,
                VerticalEdge::None,
            ),
        ] {
            let config: UserConfig = toml::from_str(source).unwrap();
            assert_eq!(config.focus.vertical_edge, expected);
        }

        // A config that never mentions `[focus]` keeps wrapping to the
        // opposite edge of the tag at the top and bottom boundaries.
        let default: UserConfig = toml::from_str("").unwrap();
        assert_eq!(default.focus.vertical_edge, VerticalEdge::Wrap);

        // `overflow` names the workspace switch, and tags do not stack
        // vertically, so it must not be selectable on this axis.
        assert!(
            toml::from_str::<UserConfig>("[focus]\nvertical_edge = \"overflow\"").is_err(),
            "vertical_edge must not accept overflow"
        );
        assert!(toml::from_str::<UserConfig>("[focus]\nvertical_edge = \"bounce\"").is_err());
    }

    #[test]
    fn bar_tag_slots_are_validated_by_field_name() {
        for (source, field) in [
            ("[bar]\ntag_slots = 0", "bar.tag_slots"),
            (
                &format!("[bar]\ntag_slots = {}", crate::types::SCRATCHPAD_TAG + 1),
                "bar.tag_slots",
            ),
            ("[monitors.DP-1]\ntag_slots = 0", "monitors.DP-1.tag_slots"),
        ] {
            let user: UserConfig = toml::from_str(source).unwrap();
            let monitor_error = user
                .monitors
                .iter()
                .find_map(|(key, entry)| entry.validated(key).err());
            let error = monitor_error
                .or_else(|| user.bar.validated().err())
                .unwrap_or_else(|| panic!("{source} should have been rejected"));
            assert!(error.contains(field), "{error}");
        }
    }

    #[test]
    fn tags_default_to_numbered_names_without_icons() {
        let tags = parse("").tags;
        assert_eq!(tags.count, crate::types::SCRATCHPAD_TAG);
        assert!(tags.names.is_empty());
        assert!(tags.icons.is_empty());
        assert_eq!(
            tags.tag_template()
                .iter()
                .map(|tag| tag.name.clone())
                .collect::<Vec<_>>(),
            (1..=20).map(|index| index.to_string()).collect::<Vec<_>>()
        );
        assert!(!tags.show_icons);
    }

    #[test]
    fn tag_names_and_icons_parse_positionally() {
        let tags = parse(
            r#"
            [tags]
            count = 3
            names = ["web", "mail", "code"]
            icons = ["W", "", "C"]
            show_icons = true
            "#,
        )
        .tags;

        let template = tags.tag_template();
        assert_eq!(template.len(), 3);
        assert_eq!(
            template
                .iter()
                .map(|tag| (tag.name.as_str(), tag.icon.as_str()))
                .collect::<Vec<_>>(),
            vec![("web", "W"), ("mail", ""), ("code", "C")]
        );
        assert!(tags.show_icons);
    }

    #[test]
    fn a_short_icon_list_leaves_the_remaining_tags_without_one() {
        let tags = parse(
            r#"
            [tags]
            count = 3
            names = ["a", "b", "c"]
            icons = ["A"]
            "#,
        )
        .tags
        .validated()
        .unwrap();

        let icons: Vec<_> = tags
            .tag_template()
            .iter()
            .map(|tag| tag.icon.clone())
            .collect();
        assert_eq!(icons, vec!["A", "", ""]);
    }

    #[test]
    fn count_is_independent_of_labels_and_omitted_icons() {
        let tags = parse("[tags]\ncount = 4\nnames = [\"web\", \"mail\"]")
            .tags
            .validated()
            .unwrap();
        let template = tags.tag_template();
        assert_eq!(
            template
                .iter()
                .map(|tag| tag.name.as_str())
                .collect::<Vec<_>>(),
            vec!["web", "mail", "3", "4"]
        );
        assert!(template.iter().all(|tag| tag.icon.is_empty()));
    }

    #[test]
    fn obsolete_empty_tag_switch_is_rejected() {
        assert!(toml::from_str::<UserConfig>("[bar]\nshow_empty_tags = false").is_err());
        assert!(toml::from_str::<UserConfig>("[monitors.DP-1]\nshow_empty_tags = false").is_err());
    }

    #[test]
    fn tag_validation_rejects_impossible_tag_sets_by_field_name() {
        for (source, field) in [
            ("[tags]\ncount = 0", "tags.count"),
            ("[tags]\ncount = 21", "tags.count"),
            ("[tags]\ncount = 1\nnames = [\"a\", \"b\"]", "tags.names"),
            ("[tags]\nnames = [\"\"]", "tags.names[0]"),
            (
                "[tags]\nnames = [\"aaaaaaaaaaaaaaaaaaaaa\"]",
                "tags.names[0]",
            ),
            (
                "[tags]\ncount = 1\nnames = [\"a\"]\nicons = [\"\", \"\"]",
                "tags.icons",
            ),
        ] {
            let user: UserConfig = toml::from_str(source).unwrap();
            let error = user.tags.validated().unwrap_err();
            assert!(error.contains(field), "{error}");
        }
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
        assert!(default.enabled);
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
    fn cli_names_match_serde_names_for_every_variant() {
        for theme in <ColorTheme as clap::ValueEnum>::value_variants() {
            let value = toml::Value::String(theme.name());
            assert_eq!(value.try_into::<ColorTheme>().unwrap(), *theme);
        }
    }

    #[test]
    fn variable_refresh_rate_is_opt_in() {
        assert_eq!(VrrMode::default(), VrrMode::Off);
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

#[cfg(test)]
mod include_tests {
    use super::*;

    /// Lay out a config tree on disk and return the root file.
    ///
    /// Each test gets its own directory, named after the test, so cases cannot
    /// see each other's files however the runner schedules them.
    fn tree(test: &str, files: &[(&str, &str)]) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("instantwm-includes-{}-{test}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        for (name, body) in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, body).unwrap();
        }
        root.join("config.toml")
    }

    fn load(root: &Path) -> Result<UserConfig, String> {
        let mut state = IncludeState::default();
        let merged = load_and_merge_config(root, &mut state)?;
        resolve_theme_colors(merged)?
            .try_into::<UserConfig>()
            .map_err(|error| error.to_string())
    }

    #[test]
    fn included_values_apply_and_the_including_file_wins() {
        let root = tree(
            "precedence",
            &[
                (
                    "config.toml",
                    "includes = [{ file = \"colors.toml\" }]\n[layout]\ninner_gap = 9\n",
                ),
                (
                    "colors.toml",
                    "[bar]\nheight = 30\n[layout]\ninner_gap = 3\n",
                ),
            ],
        );

        let config = load(&root).unwrap();

        // The included file contributes what the including file leaves out…
        assert_eq!(config.bar.height, 30);
        // …and loses where they overlap.
        assert_eq!(config.layout.inner_gap, 9);
    }

    #[test]
    fn later_includes_are_merged_over_earlier_ones() {
        let root = tree(
            "order",
            &[
                (
                    "config.toml",
                    "includes = [{ file = \"first.toml\" }, { file = \"second.toml\" }]\n",
                ),
                ("first.toml", "[layout]\ninner_gap = 3\nouter_gap = 1\n"),
                ("second.toml", "[layout]\ninner_gap = 7\n"),
            ],
        );

        let config = load(&root).unwrap();

        assert_eq!(config.layout.inner_gap, 7);
        // A key only one include sets still survives.
        assert_eq!(config.layout.outer_gap, 1);
    }

    #[test]
    fn relative_paths_resolve_against_the_including_file() {
        let root = tree(
            "relative",
            &[
                ("config.toml", "includes = [{ file = \"sub/a.toml\" }]\n"),
                (
                    "sub/a.toml",
                    // `b.toml` sits next to *this* file, not next to the root.
                    "includes = [{ file = \"b.toml\" }]\n[layout]\ninner_gap = 5\n",
                ),
                ("sub/b.toml", "[bar]\nheight = 44\n"),
            ],
        );

        let config = load(&root).unwrap();

        assert_eq!(config.layout.inner_gap, 5);
        assert_eq!(config.bar.height, 44);
    }

    #[test]
    fn a_file_reachable_twice_is_applied_once() {
        let root = tree(
            "diamond",
            &[
                (
                    "config.toml",
                    "includes = [{ file = \"b.toml\" }, { file = \"c.toml\" }]\n",
                ),
                (
                    "b.toml",
                    "includes = [{ file = \"shared.toml\" }]\n[[keybinds]]\nkey = \"F1\"\naction = \"close\"\n",
                ),
                (
                    "c.toml",
                    "includes = [{ file = \"shared.toml\" }]\n[[keybinds]]\nkey = \"F2\"\naction = \"close\"\n",
                ),
                (
                    "shared.toml",
                    "[[keybinds]]\nkey = \"F3\"\naction = \"close\"\n",
                ),
            ],
        );

        // A diamond is not a cycle: the shared file applies once. If it were
        // merged twice its keybind would appear twice.
        let config = load(&root).unwrap();

        let mut keys: Vec<&str> = config
            .keybinds
            .iter()
            .map(|bind| bind.key.as_str())
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["F1", "F2", "F3"]);
    }

    #[test]
    fn a_true_cycle_is_rejected_with_the_chain() {
        let root = tree(
            "cycle",
            &[
                ("config.toml", "includes = [{ file = \"b.toml\" }]\n"),
                ("b.toml", "includes = [{ file = \"c.toml\" }]\n"),
                ("c.toml", "includes = [{ file = \"b.toml\" }]\n"),
            ],
        );

        let error = load(&root).unwrap_err();

        assert!(
            error.starts_with("circular config include:"),
            "expected a cycle report, got: {error}"
        );
        // The chain names the whole loop, so the offending file is obvious.
        assert!(error.contains("b.toml"), "{error}");
        assert!(error.contains("c.toml"), "{error}");
    }

    #[test]
    fn a_file_including_itself_is_rejected() {
        let root = tree(
            "self",
            &[("config.toml", "includes = [{ file = \"config.toml\" }]\n")],
        );

        assert!(
            load(&root)
                .unwrap_err()
                .starts_with("circular config include:")
        );
    }

    #[test]
    fn a_bare_path_string_is_not_accepted() {
        // The obvious shorthand, and the easiest way to typo silently. It is
        // rejected outright rather than quietly ignored.
        let root = tree(
            "shorthand",
            &[("config.toml", "includes = [\"colors.toml\"]\n")],
        );

        let error = load(&root).unwrap_err();

        assert!(error.contains("invalid `includes`"), "{error}");
    }

    #[test]
    fn unknown_include_keys_are_rejected() {
        let root = tree(
            "unknown-key",
            &[(
                "config.toml",
                "includes = [{ file = \"colors.toml\", if_exists = true }]\n",
            )],
        );

        let error = load(&root).unwrap_err();

        assert!(error.contains("invalid `includes`"), "{error}");
        assert!(error.contains("if_exists"), "{error}");
    }

    #[test]
    fn a_missing_include_names_the_file_that_asked_for_it() {
        let root = tree(
            "missing",
            &[("config.toml", "includes = [{ file = \"typo.toml\" }]\n")],
        );

        let error = load(&root).unwrap_err();

        assert!(error.contains("includes[0]"), "{error}");
        assert!(error.contains("typo.toml"), "{error}");
        // The including file, so the report points at the line to fix.
        assert!(error.contains("config.toml"), "{error}");
    }

    #[test]
    fn includes_are_a_directive_and_never_reach_the_config() {
        let root = tree(
            "directive",
            &[
                ("config.toml", "includes = [{ file = \"colors.toml\" }]\n"),
                ("colors.toml", "[bar]\nheight = 30\n"),
            ],
        );

        let mut state = IncludeState::default();
        let merged = load_and_merge_config(&root, &mut state).unwrap();

        // Consumed on the way in, so it cannot be observed as a value…
        assert!(merged.get("includes").is_none());
        // …and therefore never printed back out as a phantom setting.
        let template = generate_commented_config();
        assert!(!template.contains("includes = []"), "{template}");
        // The directive is still discoverable, with its real syntax.
        assert!(template.contains("includes = [{ file ="), "{template}");
    }

    #[test]
    fn a_tilde_expands_to_the_home_directory() {
        let home = home_dir().unwrap();
        let including = Path::new("/etc/instantwm/config.toml");

        assert_eq!(
            IncludeSpec {
                file: "~/colors.toml".into()
            }
            .resolve(including)
            .unwrap(),
            home.join("colors.toml")
        );
        // A bare `~` is the directory itself, not a path under a literal `~`.
        assert_eq!(
            IncludeSpec { file: "~".into() }.resolve(including).unwrap(),
            home
        );
        // Nested and absolute-in-home paths both work.
        assert_eq!(
            IncludeSpec {
                file: "~/dotfiles/iwm/colors.toml".into()
            }
            .resolve(including)
            .unwrap(),
            home.join("dotfiles/iwm/colors.toml")
        );
    }

    #[test]
    fn another_users_home_directory_is_rejected() {
        // `~root/x` names another account, which needs a passwd lookup this does
        // not do. It must not resolve to a literal directory called `~root`.
        let error = IncludeSpec {
            file: "~root/colors.toml".into(),
        }
        .resolve(Path::new("/etc/instantwm/config.toml"))
        .unwrap_err();

        assert!(error.contains("~root/colors.toml"), "{error}");
        assert!(error.contains("'~/'"), "{error}");
    }

    #[test]
    fn non_tilde_paths_keep_their_own_rules() {
        // Absolute paths are taken as written; relative ones resolve against the
        // including file, never the process working directory.
        assert_eq!(
            IncludeSpec {
                file: "/etc/iwm/colors.toml".into()
            }
            .resolve(Path::new("/home/u/.config/instantwm/config.toml"))
            .unwrap(),
            Path::new("/etc/iwm/colors.toml")
        );
        assert_eq!(
            IncludeSpec {
                file: "colors.toml".into()
            }
            .resolve(Path::new("/home/u/.config/instantwm/config.toml"))
            .unwrap(),
            Path::new("/home/u/.config/instantwm/colors.toml")
        );
        // A `~` that is not the whole first component is an ordinary name.
        assert_eq!(
            IncludeSpec {
                file: "backup~/colors.toml".into()
            }
            .resolve(Path::new("/cfg/config.toml"))
            .unwrap(),
            Path::new("/cfg/backup~/colors.toml")
        );
    }

    #[test]
    fn an_expanded_tilde_reaches_the_loader() {
        let root = tree(
            "tilde",
            &[(
                "config.toml",
                "includes = [{ file = \"~/instantwm-include-tilde-probe.toml\" }]\n",
            )],
        );

        let error = load(&root).unwrap_err();

        // Expansion happened before the existence check, so the report names the
        // real path rather than a literal `~/…`.
        assert!(!error.contains('~'), "{error}");
        assert!(
            error.contains("instantwm-include-tilde-probe.toml"),
            "{error}"
        );
        assert!(
            error.contains(&home_dir().unwrap().display().to_string()),
            "{error}"
        );
    }
}
