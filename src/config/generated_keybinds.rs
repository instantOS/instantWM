//! Default commands resolved against the installed system when the
//! configuration is built.

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::backend::BackendKind;

const TERMINAL_CANDIDATES: &[&str] = &["kitty", "ghostty", "wezterm", "xterm", "st"];
const WAYLAND_LOCKSCREEN_CANDIDATES: &[&str] = &[
    "hyprlock",
    "swaylock",
    "gtklock",
    "waylock",
    "slock",
    "instantlock",
];
const X11_LOCKSCREEN_CANDIDATES: &[&str] = &["slock", "instantlock", "i3lock", "xlock"];

/// Application launcher bound to Super+Space.
pub fn backend_launcher(backend: BackendKind) -> &'static str {
    match backend {
        BackendKind::Wayland => "fuzzel",
        BackendKind::X11 => "instantmenu_smartrun",
    }
}

pub fn resolve_lockscreen_command(backend: BackendKind) -> &'static str {
    let default_script = ".config/instantos/default/lockscreen";
    if command_exists(default_script) {
        return default_script;
    }

    let candidates = match backend {
        BackendKind::Wayland => WAYLAND_LOCKSCREEN_CANDIDATES,
        BackendKind::X11 => X11_LOCKSCREEN_CANDIDATES,
    };

    first_installed_command(candidates, command_exists).unwrap_or(default_script)
}

/// Terminal bound to Super+Return.
pub fn resolve_terminal_command() -> &'static str {
    let default_script = ".config/instantos/default/terminal";
    if command_exists(default_script) {
        return default_script;
    }

    first_installed_command(TERMINAL_CANDIDATES, command_exists).unwrap_or(TERMINAL_CANDIDATES[0])
}

fn first_installed_command(
    candidates: &[&'static str],
    mut exists: impl FnMut(&str) -> bool,
) -> Option<&'static str> {
    candidates
        .iter()
        .copied()
        .find(|candidate| exists(candidate))
}

pub fn command_exists(command: &str) -> bool {
    if command.contains('/') {
        if (command.starts_with(".config/") || command.starts_with("~/"))
            && let Some(home) = env::var_os("HOME")
        {
            let rel = command.strip_prefix("~/").unwrap_or(command);
            let expanded = Path::new(&home).join(rel);
            if is_executable(&expanded) {
                return true;
            }
        }
        return is_executable(Path::new(command));
    }

    let Some(path) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&path).any(|dir| is_executable(&dir.join(command)))
}

fn is_executable(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata.permissions().mode() & 0o111 != 0,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_installed_command_uses_first_available_candidate() {
        let terminal =
            first_installed_command(&["wezterm", "xterm"], |candidate| candidate == "xterm");
        assert_eq!(terminal, Some("xterm"));
    }
}
