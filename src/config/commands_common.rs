//! Common command strings used across keybindings and buttons.

/// instantOS default application paths.
pub mod defaults {
    pub const FILEMANAGER: &[&str] = &[".config/instantos/default/filemanager"];
    pub const TERM_FILEMANAGER: &[&str] = &[".config/instantos/default/termfilemanager"];
    pub const APPMENU: &[&str] = &[".config/instantos/default/appmenu"];
    pub const LOCKSCREEN: &[&str] = &[".config/instantos/default/lockscreen"];
    pub const BROWSER: &[&str] = &[".config/instantos/default/browser"];
    pub const EDITOR: &[&str] = &[".config/instantos/default/editor"];
    pub const SYSTEMMONITOR: &[&str] = &[".config/instantos/default/systemmonitor"];
}

/// Volume and brightness controls.
pub mod media {
    pub const P: &[&str] = &["ins", "assist", "volume"];
    pub const B: &[&str] = &["ins", "assist", "bright"];

    pub fn up_vol() -> &'static [&'static str] {
        &["ins", "assist", "volume", "+"]
    }
    pub fn down_vol() -> &'static [&'static str] {
        &["ins", "assist", "volume", "-"]
    }
    pub fn mute_vol() -> &'static [&'static str] {
        &["ins", "assist", "volume", "mute"]
    }
    pub fn up_bright() -> &'static [&'static str] {
        &["ins", "assist", "bright", "+"]
    }
    pub fn down_bright() -> &'static [&'static str] {
        &["ins", "assist", "bright", "-"]
    }
}

/// Screenshot utilities.
pub mod scrot {
    pub const S: &[&str] = &["ins", "assist", "run", "sp"];
    pub const M: &[&str] = &["ins", "assist", "run", "sm"];
    pub const C: &[&str] = &["ins", "assist", "run", "sc"];
    pub const F: &[&str] = &["ins", "assist", "run", "sf"];
}

/// Rofi window switcher (for iswitch-style window switching).
pub const ROFI_WINDOW_SWITCH: &[&str] = &[
    "rofi",
    "-show",
    "window",
    "-kb-row-down",
    "Alt+Tab,Down",
    "-kb-row-up",
    "Alt+Ctrl+Tab,Up",
    "-kb-accept-entry",
    "!Alt_L,!Alt+Tab,Return",
    "-me-select-entry",
    "",
    "-me-accept-entry",
    "MousePrimary",
    "-theme",
    "/usr/share/instantdotfiles/rootconfig/rofi/appmenu.rasi",
];

/// Shortcuts for common instantmenu variants.
pub mod menu {
    pub const RUN: &[&str] = &["instantmenu_run"];
    pub const SMART: &[&str] = &["instantmenu_smartrun"];
    pub const ST: &[&str] = &["instantmenu_run_st"];
    pub const CLIP: &[&str] = &["instantclipmenu"];
    pub const QUICK: &[&str] = &["quickmenu"];
}
