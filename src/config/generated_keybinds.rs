use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::backend::BackendKind;
use crate::config::config_toml::ThemeConfig;
use crate::config::keybind_config::{
    ActionSpec, KeybindSpec, StructuredAction, merge_keybinds, parse_keysym, parse_modifiers,
};
use crate::config::keybindings::{MODKEY, get_desktop_keybinds, get_keys};
use crate::config::keysyms::{XK_RETURN, XK_SPACE};
use crate::types::Key;

const TERMINAL_CANDIDATES: &[&str] = &["kitty", "ghostty", "wezterm", "xterm", "st"];

pub struct DefaultKeybinds {
    pub keys: Vec<Key>,
    pub desktop_keybinds: Vec<Key>,
}

pub fn build_default_keybinds(backend: BackendKind, theme: &ThemeConfig) -> DefaultKeybinds {
    let generated_keys = build_generated_keybind_specs(backend, &theme.keybinds);

    DefaultKeybinds {
        keys: merge_keybinds(get_keys(), &generated_keys),
        desktop_keybinds: get_desktop_keybinds(),
    }
}

fn build_generated_keybind_specs(
    backend: BackendKind,
    user_keybinds: &[KeybindSpec],
) -> Vec<KeybindSpec> {
    let mut specs = Vec::new();

    if !has_override(user_keybinds, MODKEY, XK_RETURN) {
        specs.push(spawn_keybind_spec(
            vec!["super".to_string()],
            "return",
            vec![resolve_terminal_command().to_string()],
        ));
    }

    if !has_override(user_keybinds, MODKEY, XK_SPACE) {
        specs.push(spawn_keybind_spec(
            vec!["super".to_string()],
            "space",
            vec![backend_launcher(backend).to_string()],
        ));
    }

    specs
}

fn spawn_keybind_spec(modifiers: Vec<String>, key: &str, command: Vec<String>) -> KeybindSpec {
    KeybindSpec {
        modifiers,
        key: key.to_string(),
        action: ActionSpec::Structured(StructuredAction::Spawn(command)),
    }
}

fn has_override(specs: &[KeybindSpec], mod_mask: u32, keysym: u32) -> bool {
    specs.iter().any(|spec| {
        let Some(spec_mod_mask) = parse_modifiers(&spec.modifiers) else {
            return false;
        };
        let Some(spec_keysym) = parse_keysym(&spec.key) else {
            return false;
        };
        spec_mod_mask == mod_mask && spec_keysym == keysym
    })
}

fn backend_launcher(backend: BackendKind) -> &'static str {
    match backend {
        BackendKind::Wayland => "fuzzel",
        BackendKind::X11 => "instantmenu_smartrun",
    }
}

fn resolve_terminal_command() -> &'static str {
    first_installed_terminal(command_exists).unwrap_or(TERMINAL_CANDIDATES[0])
}

fn first_installed_terminal(mut exists: impl FnMut(&str) -> bool) -> Option<&'static str> {
    TERMINAL_CANDIDATES
        .iter()
        .copied()
        .find(|candidate| exists(candidate))
}

fn command_exists(command: &str) -> bool {
    if command.contains('/') {
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
    use crate::actions::{KeyAction, NamedAction};

    fn spawn_args_for(keys: &[Key], mod_mask: u32, keysym: u32) -> Option<&[String]> {
        keys.iter().find_map(|key| {
            if key.mod_mask != mod_mask || key.keysym != keysym {
                return None;
            }

            match &key.action {
                KeyAction::Named {
                    action: NamedAction::Spawn,
                    args,
                } => Some(args.as_slice()),
                _ => None,
            }
        })
    }

    #[test]
    fn first_installed_terminal_uses_first_available_candidate() {
        let terminal =
            first_installed_terminal(|candidate| matches!(candidate, "wezterm" | "xterm"));
        assert_eq!(terminal, Some("wezterm"));
    }

    #[test]
    fn generated_super_enter_is_skipped_when_user_overrides_it() {
        let mut theme = ThemeConfig::default();
        theme.keybinds.push(KeybindSpec {
            modifiers: vec!["super".to_string()],
            key: "return".to_string(),
            action: ActionSpec::Structured(StructuredAction::Spawn(vec!["alacritty".to_string()])),
        });

        let defaults = build_default_keybinds(BackendKind::X11, &theme);
        let args = spawn_args_for(&defaults.keys, MODKEY, XK_RETURN).expect("missing super+enter");

        assert_eq!(args, &["kitty".to_string()]);
    }

    #[test]
    fn generated_super_space_depends_on_backend() {
        let x11 = build_default_keybinds(BackendKind::X11, &ThemeConfig::default());
        let wayland = build_default_keybinds(BackendKind::Wayland, &ThemeConfig::default());

        assert_eq!(
            spawn_args_for(&x11.keys, MODKEY, XK_SPACE),
            Some(&["instantmenu_smartrun".to_string()][..]),
        );
        assert_eq!(
            spawn_args_for(&wayland.keys, MODKEY, XK_SPACE),
            Some(&["fuzzel".to_string()][..]),
        );
    }
}
