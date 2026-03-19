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
//! - **Tune WM parameters** (border width, mfact, …) → [`Config`] defaults below

pub mod appearance;
pub mod buttons;
pub mod commands;
pub mod commands_common;
pub mod config_toml;
pub mod keybind_config;
pub mod keybindings;
pub mod keysyms;
pub mod rules;

// Re-export modifier key constants (used by wayland/common.rs via crate::config::*).
pub use crate::types::{SchemeClose, SchemeHover, SchemeTag, SchemeWin};
pub use keybindings::{CONTROL, MOD1, MODKEY, SHIFT};

use commands::{ExternalCommands, default_commands};
use keybindings::{get_desktop_keybinds, get_keys};
use mod_consts::BORDERPX;

// ---------------------------------------------------------------------------
// Module-level constants
// ---------------------------------------------------------------------------

/// Shared constants referenced by multiple sub-modules.
pub mod mod_consts {
    use crate::types::MAX_TAGS;

    /// Default border width in pixels.
    pub const BORDERPX: i32 = 3;

    /// Maximum tag name length.
    pub const MAX_TAGLEN: usize = 16;

    /// Bitmask covering all valid tags.
    pub const TAGMASK: u32 = (1 << MAX_TAGS) - 1;
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

/// Mode configuration with keybinds and optional description.
#[derive(Debug, Clone, Default)]
pub struct ModeConfig {
    /// Optional description shown in status bar when mode is active.
    pub description: Option<String>,
    /// Keybinds for this mode.
    pub keybinds: Vec<Key>,
}

/// All WM configuration in one place.
///
/// Built by [`init_config`] and consumed by `init_globals` in `startup::x11`.
/// Fields are public so `init_globals` can move them into `Globals` without
/// extra getters.
#[derive(Debug, Clone)]
pub struct Config {
    // --- Window geometry ---
    /// Border width in pixels.
    pub borderpx: i32,
    /// Snap-to-edge distance in pixels.
    pub snap: i32,

    // --- Bar / systray ---
    /// Start menu button width in pixels.
    pub startmenusize: i32,
    /// Index of monitor to pin the systray to (0 = primary).
    pub systraypinning: usize,
    /// Gap in pixels between systray icons.
    pub systrayspacing: i32,
    /// If systray pinning fails, place it on the first monitor.
    pub systraypinningfailfirst: bool,
    /// Whether to show the systray.
    pub showsystray: bool,
    /// Whether to show the bar by default.
    pub showbar: bool,
    /// `true` = bar at top, `false` = bar at bottom.
    pub topbar: bool,
    /// Override bar height (0 = derive from font metrics).
    pub bar_height: i32,

    // --- Tiling ---
    /// Respect size hints for tiled clients (`1` = yes).
    pub resizehints: i32,
    /// Respect decoration hints (`1` = yes).
    pub decorhints: i32,
    /// Master area size factor (0.0–1.0).
    pub mfact: f32,
    /// Number of clients in master area.
    pub nmaster: i32,

    // --- Tags ---
    pub tag_names: Vec<String>,
    pub tag_alt_names: Vec<String>,
    /// Color table for tag buttons: `[hover][SchemeTag]`
    pub tag_colors: TagColorConfigs,
    pub num_tags: usize,

    // --- Color tables ---
    /// `[hover][SchemeWin]`
    pub windowcolors: WindowColorConfigs,
    /// `[hover][SchemeClose]`
    pub closebuttoncolors: CloseButtonColorConfigs,
    /// `[SchemeBorder as usize]`
    pub bordercolors: BorderColorConfig,
    /// Status bar colors (fg, bg, detail)
    pub statusbarcolors: StatusColorConfig,

    // --- Bindings ---
    pub keys: Vec<Key>,
    pub desktop_keybinds: Vec<Key>,
    pub modes: std::collections::HashMap<String, ModeConfig>,
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
    pub input: std::collections::HashMap<String, config_toml::InputConfig>,
    /// Monitor configuration.
    pub monitors: std::collections::HashMap<String, config_toml::MonitorConfig>,
    pub status_command: Option<String>,
}

// ---------------------------------------------------------------------------
// init_config
// ---------------------------------------------------------------------------

/// Build the default [`Config`].
///
/// Called once from `init_globals` in `startup::x11`.  All values here are the
/// compile-time defaults; TOML config overrides the appearance fields when present.
pub fn init_config() -> Config {
    let theme = config_toml::load_config_file();

    // Merge TOML keybinds over compiled defaults
    let keys = if theme.keybinds.is_empty() {
        get_keys()
    } else {
        keybind_config::merge_keybinds(get_keys(), &theme.keybinds)
    };
    let desktop_keybinds = if theme.desktop_keybinds.is_empty() {
        get_desktop_keybinds()
    } else {
        keybind_config::merge_keybinds(get_desktop_keybinds(), &theme.desktop_keybinds)
    };

    let mut modes = std::collections::HashMap::new();
    for (name, spec) in &theme.modes {
        let keybinds = keybind_config::merge_keybinds(Vec::new(), &spec.keybinds);
        modes.insert(
            name.clone(),
            ModeConfig {
                description: spec.description.clone(),
                keybinds,
            },
        );
    }

    Config {
        // --- Window geometry ---
        borderpx: BORDERPX,
        snap: 32,

        // --- Bar / systray ---
        startmenusize: 30,
        systraypinning: 0,
        systrayspacing: 0,
        systraypinningfailfirst: true,
        showsystray: true,
        showbar: true,
        topbar: true,
        bar_height: 0,

        // --- Tiling ---
        resizehints: 1,
        decorhints: 1,
        mfact: 0.55,
        nmaster: 1,

        // --- Tags ---
        tag_names: get_tags(),
        tag_alt_names: get_tags_alt(),
        num_tags: MAX_TAGS,

        // --- Appearance (from TOML if present, else palette defaults) ---
        fonts: theme.fonts,
        tag_colors: theme.colors.tag,
        windowcolors: theme.colors.window,
        closebuttoncolors: theme.colors.close_button,
        bordercolors: theme.colors.border,
        statusbarcolors: theme.colors.status,

        // --- Bindings (merged with TOML overrides) ---
        keys,
        desktop_keybinds,
        modes,
        buttons: buttons::get_buttons(),
        rules: rules::get_rules(),

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
    }
}
