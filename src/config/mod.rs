//! Window manager configuration.
//!
//! This module is the single place to tune instantWM behaviour.  It is split
//! into focused sub-modules so you can find what you need quickly:
//!
//! | Module            | What lives there                                        |
//! |-------------------|---------------------------------------------------------|
//! | [`appearance`]    | Color palette, per-scheme color tables, font list       |
//! | [`commands`]      | External commands (`ExternalCommands`, `Cmd` enum)      |
//! | [`keybindings`]   | Normal-mode key bindings (`get_keys`, `get_desktop_keybinds`)      |
//! | [`buttons`]       | Mouse button bindings (`get_buttons`)                   |
//! | [`rules`]         | Window placement rules (`get_rules`)                    |
//! | [`keysyms`]       | X11 keysym constants (re-exported via `use keysyms::*`) |
//!
//! # Quick-start: changing things
//!
//! - **Add/change a keybinding** → [`keybindings`]
//! - **Add/change a mouse button** → [`buttons`]
//! - **Change colors** → [`appearance::palette`]
//! - **Add an external command** → [`commands`] (add field + `Cmd` variant)
//! - **Change a window rule** → [`rules`]
//! - **Tune WM parameters** (border width, gaps, …) → [`Config`] defaults below

pub mod appearance;
pub mod buttons;
pub mod commands;
pub mod commands_common;
pub mod config_toml;
pub mod generated_keybinds;
pub mod keybind_config;
pub mod keybindings;
pub mod keysyms;
pub mod rules;

// Re-export modifier key constants (used by wayland/common.rs via crate::config::*).
pub use crate::types::{EdgeDirection, SchemeClose, SchemeHover, SchemeTag, SchemeWin};
pub use keybindings::{CONTROL, MOD1, MODKEY, SHIFT};

use commands::{ExternalCommands, default_commands};
use mod_consts::BORDER_PX;

// ---------------------------------------------------------------------------
// Module-level constants
// ---------------------------------------------------------------------------

/// Shared constants referenced by multiple sub-modules.
pub mod mod_consts {
    use crate::types::MAX_TAGS;

    /// Default border width in pixels.
    pub const BORDER_PX: i32 = 3;

    /// Maximum tag name length.
    pub const MAX_TAGLEN: usize = 16;

    /// Bitmask covering all valid tags.
    pub const TAG_MASK: u32 = (1 << MAX_TAGS) - 1;
}

// ---------------------------------------------------------------------------
// Tag configuration
// ---------------------------------------------------------------------------

use crate::types::MAX_TAGS;

/// Default tag names (used when no config override is set).
///
/// There are [`MAX_TAGS`] entries — the last one (`"s"`) is the scratchpad tag.
pub fn get_tags_default() -> [&'static str; MAX_TAGS] {
    [
        "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
        "17", "18", "19", "20", "s",
    ]
}

/// Build the tag name list as owned `String`s.
pub fn get_tags() -> Vec<String> {
    get_tags_default().iter().map(|&s| s.to_string()).collect()
}

/// Alternative (icon) tag names shown when alt-tag mode is active.
pub fn get_tags_alt() -> Vec<String> {
    vec![
        "".to_string(),
        "{}".to_string(),
        "$".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
    ]
}

// ---------------------------------------------------------------------------
// Config struct
// ---------------------------------------------------------------------------

use crate::types::{
    BorderColorConfig, Button, CloseButtonColorConfigs, Key, Rule, StatusColorConfig,
    TagColorConfigs, WindowColorConfigs,
};
use std::collections::HashMap;

use generated_keybinds::build_default_keybinds;

/// Mode configuration with keybinds and optional description.
#[derive(Debug, Clone, Default)]
pub struct ModeConfig {
    /// Optional description shown in status bar when mode is active.
    pub description: Option<String>,
    /// Whether the mode is transient (reset to default after any keybind).
    pub transient: bool,
    /// Keybinds for this mode.
    pub keybinds: Vec<Key>,
}

/// All WM configuration in one place.
///
/// Built by [`init_config`] and converted atomically into runtime configuration
/// by `core_state::apply_config`.
#[derive(Debug, Clone)]
pub struct Config {
    // --- Window geometry ---
    /// Border width in pixels.
    pub border_px: i32,
    /// Snap-to-edge distance in pixels.
    pub snap_threshold: i32,

    // --- Bar / systray ---
    /// Start menu button width in pixels.
    pub startmenu_size: i32,
    /// Index of monitor to pin the systray to (0 = primary).
    pub systray_pinning: usize,
    /// Desired breathing room around tray icons. StatusNotifier items use visual
    /// padding inside contiguous input cells; legacy XEmbed uses literal spacing.
    pub systray_spacing: i32,
    /// If systray pinning fails, place it on the first monitor.
    pub systray_pinning_fail_first: bool,
    /// Whether to show the systray.
    pub show_systray: bool,
    /// Whether to show the bar by default.
    pub show_bar: bool,
    /// Position of the status bar (top, bottom, left, right).
    pub bar_position: EdgeDirection,
    /// Override bar height (0 = derive from font metrics).
    pub bar_height: i32,

    // --- Tiling ---
    /// Respect size hints for tiled clients.
    pub resize_hints: bool,
    /// Respect decoration hints.
    pub decor_hints: bool,
    /// Tiled layout gap configuration.
    pub layout: config_toml::LayoutConfig,

    // --- Tags ---
    pub tag_names: Vec<String>,
    pub tag_alt_names: Vec<String>,
    /// Color table for tag buttons: `[hover][SchemeTag]`
    pub tag_colors: TagColorConfigs,
    pub num_tags: usize,

    // --- Color tables ---
    /// Active built-in colour theme (the base the `*_colors` tables derive from).
    pub theme: config_toml::ColorTheme,
    /// `[hover][SchemeWin]`
    pub window_colors: WindowColorConfigs,
    /// `[hover][SchemeClose]`
    pub closebuttoncolors: CloseButtonColorConfigs,
    /// `[SchemeBorder as usize]`
    pub border_colors: BorderColorConfig,
    /// Status bar colors (fg, bg, detail)
    pub statusbarcolors: StatusColorConfig,

    // --- Bindings ---
    pub keys: Vec<Key>,
    pub desktop_keybinds: Vec<Key>,
    pub modes: HashMap<String, ModeConfig>,
    pub buttons: Vec<Button>,
    pub rules: Vec<Rule>,
    pub fonts: Vec<String>,

    // --- External commands ---
    pub external_commands: ExternalCommands,

    // --- Keyboard layouts ---
    /// XKB keyboard layouts.
    pub keyboard_layouts: Vec<config_toml::KeyboardLayoutConfig>,
    /// XKB options string.
    pub keyboard_options: Option<String>,
    /// XKB model string.
    pub keyboard_model: Option<String>,
    /// Swap Caps Lock and Escape.
    pub keyboard_swapescape: bool,

    // --- Input configuration ---
    pub input: HashMap<String, config_toml::InputConfig>,
    /// Monitor configuration.
    pub monitors: HashMap<String, config_toml::MonitorConfig>,
    pub status_command: Option<String>,
    pub cursor: config_toml::CursorConfig,

    // --- Exec commands ---
    /// Commands to execute once at startup.
    pub exec_once: Vec<String>,
    /// Commands to execute at startup and on every config reload.
    pub exec: Vec<String>,
}

// ---------------------------------------------------------------------------
// init_config
// ---------------------------------------------------------------------------

/// Build the default [`Config`].
///
/// Called once from `init_globals` in `backend::x11::startup`.  All values here are the
/// compile-time defaults; TOML config overrides the appearance fields when present.
pub fn init_config(backend: crate::backend::BackendKind) -> Config {
    let theme = config_toml::load_config_file();
    let defaults = build_default_keybinds(backend, &theme);

    // Merge TOML keybinds over compiled defaults
    let keys = if theme.keybinds.is_empty() {
        defaults.keys
    } else {
        keybind_config::merge_keybinds(defaults.keys, &theme.keybinds)
    };
    let desktop_keybinds = if theme.desktop_keybinds.is_empty() {
        defaults.desktop_keybinds
    } else {
        keybind_config::merge_keybinds(defaults.desktop_keybinds, &theme.desktop_keybinds)
    };

    let mut modes = HashMap::new();

    // Helper for merging mode keybinds
    let merge_mode = |spec: Option<&config_toml::ModeSpec>,
                      default_desc: &str,
                      default_transient: bool,
                      default_keybinds: Vec<Key>|
     -> ModeConfig {
        if let Some(spec) = spec {
            let keybinds = keybind_config::merge_keybinds(default_keybinds, &spec.keybinds);
            ModeConfig {
                description: spec
                    .description
                    .clone()
                    .or_else(|| Some(default_desc.to_string())),
                transient: spec.transient.unwrap_or(default_transient),
                keybinds,
            }
        } else {
            ModeConfig {
                description: Some(default_desc.to_string()),
                transient: default_transient,
                keybinds: default_keybinds,
            }
        }
    };

    // Special handling for default modes: prefix and desktop
    modes.insert(
        "prefix".to_string(),
        merge_mode(theme.modes.get("prefix"), "prefix", true, Vec::new()),
    );

    modes.insert(
        "desktop".to_string(),
        merge_mode(theme.modes.get("desktop"), "desktop", false, Vec::new()),
    );

    let mut placement_mode = merge_mode(
        theme.modes.get(crate::core_state::TREE_PLACEMENT_MODE_NAME),
        "place window",
        false,
        keybindings::get_tree_placement_keybinds(),
    );
    // Placement has a transactional apply/cancel lifecycle; treating one
    // command as transient would discard that transaction mid-navigation.
    placement_mode.transient = false;
    modes.insert(
        crate::core_state::TREE_PLACEMENT_MODE_NAME.to_string(),
        placement_mode,
    );

    // Add all other user-defined modes
    for (name, spec) in &theme.modes {
        if name == "prefix"
            || name == "desktop"
            || name == crate::core_state::TREE_PLACEMENT_MODE_NAME
        {
            continue;
        }
        let keybinds = keybind_config::merge_keybinds(Vec::new(), &spec.keybinds);
        modes.insert(
            name.clone(),
            ModeConfig {
                description: spec.description.clone(),
                transient: spec.transient.unwrap_or(false),
                keybinds,
            },
        );
    }

    Config {
        // --- Window geometry ---
        border_px: BORDER_PX,
        snap_threshold: 32,

        // --- Bar / systray ---
        startmenu_size: 30,
        systray_pinning: 0,
        systray_spacing: 0,
        systray_pinning_fail_first: true,
        show_systray: true,
        show_bar: true,
        bar_position: EdgeDirection::Top,
        bar_height: theme.bar_height as i32,

        // --- Tiling ---
        resize_hints: true,
        decor_hints: true,
        layout: theme.layout,

        // --- Tags ---
        tag_names: get_tags(),
        tag_alt_names: get_tags_alt(),
        num_tags: MAX_TAGS,

        // --- Appearance (from TOML if present, else palette defaults) ---
        fonts: theme.fonts,
        theme: theme.theme,
        tag_colors: theme.colors.tag,
        window_colors: theme.colors.window,
        closebuttoncolors: theme.colors.close_button,
        border_colors: theme.colors.border,
        statusbarcolors: theme.colors.status,

        // --- Bindings (merged with TOML overrides) ---
        keys,
        desktop_keybinds,
        modes,
        buttons: buttons::get_buttons(),
        rules: rules::merge_rules(rules::get_rules(), theme.rules),

        // --- External commands ---
        external_commands: default_commands(),

        // --- Keyboard layouts ---
        keyboard_layouts: theme.keyboard.layouts.clone(),
        keyboard_options: theme.keyboard.options.clone(),
        keyboard_model: theme.keyboard.model.clone(),
        keyboard_swapescape: theme.keyboard.swapescape,

        // --- Input configuration ---
        input: theme.input.clone(),
        monitors: theme.monitors.clone(),
        status_command: theme.status_command.clone(),
        cursor: theme.cursor.clone(),

        // --- Exec commands ---
        exec_once: theme.exec_once.clone(),
        exec: theme.exec.clone(),
    }
}
