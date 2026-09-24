//! TOML-configurable keybindings.
//!
//! Parses `[[keybinds]]` and `[[desktop_keybinds]]` entries from the config
//! file and merges them with the compiled defaults. TOML entries override
//! defaults where `(mod_mask, keysym)` matches; unmatched entries are appended.
//!
//! ```toml
//! [[keybinds]]
//! modifiers = ["super"]
//! key = "Return"
//! action = ["spawn", "kitty"]        # name followed by its arguments
//!
//! [[keybinds]]
//! modifiers = ["super"]
//! key = "q"
//! action = "none"                    # remove the default binding
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use smithay::input::keyboard::xkb;

use crate::actions::{KeyAction, NamedAction};
use crate::config::keybindings::{CONTROL, MOD1, MODKEY, SHIFT};
use crate::types::{Key, KeybindOrigin};

/// A single keybind entry from the TOML config.
#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct KeybindSpec {
    #[serde(default)]
    pub modifiers: Vec<String>,
    pub key: String,
    pub action: ActionSpec,
}

/// A configured action: an action name (`"zoom"`, or `"none"` to unbind), a
/// name followed by its arguments (`["set_layout", "grid"]`), or a sequence.
#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(untagged)]
pub enum ActionSpec {
    Named(String),
    WithArgs(Vec<String>),
    Sequence { sequence: Vec<ActionSpec> },
}

impl ActionSpec {
    /// `"none"` removes a binding instead of running anything.
    fn is_unbind(&self) -> bool {
        matches!(self, Self::Named(name) if name == "none")
    }
}

pub fn parse_modifiers(mods: &[String]) -> Result<u32, String> {
    mods.iter().try_fold(0, |mask, m| {
        Ok(mask
            | match m.to_ascii_lowercase().as_str() {
                "super" | "mod" | "mod4" | "modkey" => MODKEY,
                "shift" => SHIFT,
                "control" | "ctrl" => CONTROL,
                "alt" | "mod1" => MOD1,
                "" => 0,
                other => return Err(format!("unknown modifier '{other}'")),
            })
    })
}

/// Resolve a key name: any XKB keysym name (case-insensitive, e.g. `Return`,
/// `bracketleft`, `XF86AudioMute`), a single character (`-`, `/`), or one of
/// a few short aliases.
pub fn parse_keysym(name: &str) -> Result<u32, String> {
    let mut chars = name.chars();
    if let (Some(ch), None) = (chars.next(), chars.next())
        && !ch.is_alphanumeric()
    {
        let keysym = xkb::utf32_to_keysym(ch as u32).raw();
        if keysym != 0 {
            return Ok(keysym);
        }
    }
    let name = match name.to_ascii_lowercase().as_str() {
        "enter" => "Return",
        "esc" => "Escape",
        "pageup" => "Prior",
        "pagedown" => "Next",
        _ => name,
    };
    match xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE).raw() {
        0 => Err(format!("unknown key name '{name}'")),
        keysym => Ok(keysym),
    }
}

/// Compile an action spec into an executable action.
///
/// Shared by keybinds and event hooks so both accept exactly the same action
/// vocabulary. `"none"` is a binding-table directive, not an executable
/// action, and is therefore rejected here.
pub(crate) fn compile_action(spec: &ActionSpec) -> Result<KeyAction, String> {
    match spec {
        ActionSpec::Named(name) if spec.is_unbind() => {
            Err(format!("'{name}' is not an executable action"))
        }
        ActionSpec::Named(name) => NamedAction::parse(name, &[]).map(KeyAction::Named),
        ActionSpec::WithArgs(parts) => match parts.split_first() {
            Some((name, args)) => NamedAction::parse(name, args).map(KeyAction::Named),
            None => Err("action must not be empty".to_string()),
        },
        ActionSpec::Sequence { sequence } => {
            if sequence.is_empty() {
                return Err("'sequence' must contain at least one action".to_string());
            }
            sequence
                .iter()
                .map(compile_action)
                .collect::<Result<_, _>>()
                .map(KeyAction::Sequence)
        }
    }
}

fn compile_keybind(spec: &KeybindSpec) -> Result<((u32, u32), Option<KeyAction>), String> {
    let combo = (parse_modifiers(&spec.modifiers)?, parse_keysym(&spec.key)?);
    if spec.action.is_unbind() {
        return Ok((combo, None));
    }
    Ok((combo, Some(compile_action(&spec.action)?)))
}

/// Overlay `specs` on `defaults`. Invalid entries are reported and skipped so
/// one typo does not discard the rest of the configuration.
pub fn merge_keybinds(
    defaults: Vec<Key>,
    specs: &[KeybindSpec],
    origin: KeybindOrigin,
) -> Vec<Key> {
    let mut keys: Vec<Option<Key>> = defaults.into_iter().map(Some).collect();
    let mut index: HashMap<(u32, u32), usize> = keys
        .iter()
        .flatten()
        .enumerate()
        .map(|(i, k)| ((k.mod_mask, k.keysym), i))
        .collect();

    for spec in specs {
        let ((mod_mask, keysym), action) = match compile_keybind(spec) {
            Ok(compiled) => compiled,
            Err(error) => {
                eprintln!("instantwm: ignoring keybind for '{}': {error}", spec.key);
                continue;
            }
        };
        let slot = action.map(|action| Key {
            mod_mask,
            keysym,
            action,
            origin,
        });
        match (index.get(&(mod_mask, keysym)), slot) {
            (Some(&idx), slot) => keys[idx] = slot,
            (None, Some(key)) => {
                index.insert((mod_mask, keysym), keys.len());
                keys.push(Some(key));
            }
            (None, None) => {}
        }
    }

    keys.into_iter().flatten().collect()
}

/// Render a modifier mask as `Super + Ctrl + Shift` (empty when no modifiers).
pub fn format_modifiers(mask: u32) -> String {
    [
        (MODKEY, "Super"),
        (CONTROL, "Ctrl"),
        (SHIFT, "Shift"),
        (MOD1, "Alt"),
    ]
    .into_iter()
    .filter(|(bit, _)| mask & bit != 0)
    .map(|(_, name)| name)
    .collect::<Vec<_>>()
    .join(" + ")
}

/// Render a keysym as the user would type it: printable symbols as the
/// character itself, everything else by its XKB name.
pub fn format_keysym(keysym: u32) -> String {
    let keysym = xkb::Keysym::new(keysym);
    let text = xkb::keysym_to_utf8(keysym);
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) if ch.is_ascii_graphic() => text,
        _ => xkb::keysym_get_name(keysym),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::keysyms::*;

    fn parse_keybind(source: &str) -> KeybindSpec {
        #[derive(Deserialize)]
        struct Wrapper {
            keybind: KeybindSpec,
        }

        toml::from_str::<Wrapper>(source).unwrap().keybind
    }

    fn default_key(keysym: u32) -> Key {
        Key {
            mod_mask: MOD1,
            keysym,
            action: KeyAction::named(NamedAction::Zoom),
            origin: KeybindOrigin::CompiledDefault,
        }
    }

    #[test]
    fn none_action_removes_default() {
        let spec = parse_keybind(
            r#"
            [keybind]
            modifiers = ["Mod1"]
            key = "p"
            action = "none"
            "#,
        );
        let merged = merge_keybinds(vec![default_key(XK_P)], &[spec], KeybindOrigin::User);
        assert!(merged.is_empty());
    }

    #[test]
    fn merge_keybinds_adds_and_overrides() {
        let specs = [
            parse_keybind(
                r#"
                [keybind]
                modifiers = ["Mod1"]
                key = "p"
                action = "toggle_bar"
                "#,
            ),
            parse_keybind(
                r#"
                [keybind]
                modifiers = ["alt"]
                key = "o"
                action = ["set_layout", "grid"]
                "#,
            ),
        ];

        let merged = merge_keybinds(vec![default_key(XK_P)], &specs, KeybindOrigin::User);
        assert_eq!(merged.len(), 2);
        assert!(matches!(
            merged[0].action,
            KeyAction::Named(NamedAction::ToggleBar)
        ));
        assert_eq!(merged[1].keysym, XK_O);
        assert!(matches!(
            merged[1].action,
            KeyAction::Named(NamedAction::SetLayout(crate::layouts::LayoutCommand::Grid))
        ));
        assert!(merged.iter().all(|key| key.origin == KeybindOrigin::User));
    }

    #[test]
    fn invalid_entries_are_skipped_without_touching_defaults() {
        let specs = [
            parse_keybind(
                "[keybind]\nkey = \"p\"\nmodifiers = [\"Mod1\"]\naction = \"does_not_exist\"",
            ),
            parse_keybind(
                "[keybind]\nkey = \"p\"\nmodifiers = [\"Mod1\"]\naction = [\"set_layout\"]",
            ),
            parse_keybind("[keybind]\nkey = \"nokey\"\naction = \"zoom\""),
            parse_keybind("[keybind]\nkey = \"p\"\nmodifiers = [\"hyper\"]\naction = \"zoom\""),
        ];
        let merged = merge_keybinds(vec![default_key(XK_P)], &specs, KeybindOrigin::User);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].origin, KeybindOrigin::CompiledDefault);
    }

    #[test]
    fn sequence_action_parses_and_compiles_in_order() {
        let spec = parse_keybind(
            r#"
            [keybind]
            modifiers = []
            key = "f"
            action = { sequence = [
                ["set_mode", "default"],
                ["spawn", "ins", "assist", "run", "sf"],
                { sequence = ["zoom", "quit"] },
            ] }
            "#,
        );

        let merged = merge_keybinds(Vec::new(), &[spec], KeybindOrigin::User);
        let [key] = merged.as_slice() else {
            panic!("expected one compiled keybinding");
        };
        let KeyAction::Sequence(actions) = &key.action else {
            panic!("expected a sequence action");
        };
        assert_eq!(actions.len(), 3);
        assert!(matches!(
            &actions[0],
            KeyAction::Named(NamedAction::SetMode(mode)) if mode == "default"
        ));
        assert!(matches!(
            &actions[1],
            KeyAction::Named(NamedAction::Spawn(argv)) if argv == &["ins", "assist", "run", "sf"]
        ));
        assert!(matches!(&actions[2], KeyAction::Sequence(inner) if inner.len() == 2));
    }

    #[test]
    fn sequence_action_rejects_empty_or_non_executable_members() {
        for action in [
            "{ sequence = [] }",
            r#"{ sequence = [["set_mode", "default"], "none"] }"#,
            "[]",
        ] {
            let spec = parse_keybind(&format!(
                "[keybind]\nmodifiers = []\nkey = \"f\"\naction = {action}"
            ));
            assert!(
                merge_keybinds(Vec::new(), &[spec], KeybindOrigin::User).is_empty(),
                "{action}"
            );
        }
    }

    #[test]
    fn key_names_resolve_through_xkb() {
        for (name, keysym) in [
            ("return", XK_RETURN),
            ("Return", XK_RETURN),
            ("enter", XK_RETURN),
            ("esc", XK_ESCAPE),
            ("page_up", XK_PAGE_UP),
            ("f12", XK_F12),
            ("A", XK_A),
            ("7", XK_0 + 7),
            ("dead_circumflex", XK_DEAD_CIRCUMFLEX),
            ("XF86AudioMute", XF86XK_AUDIO_MUTE),
        ] {
            assert_eq!(parse_keysym(name), Ok(keysym), "{name}");
        }
        assert!(parse_keysym("nokey").is_err());
    }

    #[test]
    fn punctuation_keysyms_parse_by_symbol_and_name_and_format_as_symbol() {
        for (sym, name, keysym) in [
            ("-", "minus", XK_MINUS),
            ("+", "plus", XK_PLUS),
            (",", "comma", XK_COMMA),
            (".", "period", XK_PERIOD),
            ("/", "slash", XK_SLASH),
            (";", "semicolon", XK_SEMICOLON),
            ("=", "equal", XK_EQUAL),
            ("[", "bracketleft", XK_BRACKET_LEFT),
            ("\\", "backslash", XK_BACKSLASH),
            ("`", "grave", XK_GRAVE),
            ("'", "apostrophe", XK_APOSTROPHE),
        ] {
            assert_eq!(parse_keysym(sym), Ok(keysym));
            assert_eq!(parse_keysym(name), Ok(keysym));
            assert_eq!(format_keysym(keysym), sym);
        }
        assert_eq!(format_keysym(XK_A), "a");
        assert_eq!(format_keysym(XK_RETURN), "Return");
        assert_eq!(format_keysym(XK_DEAD_CIRCUMFLEX), "dead_circumflex");
    }

    #[test]
    fn modifier_formatting_matches_combinations() {
        assert_eq!(format_modifiers(0), "");
        assert_eq!(format_modifiers(MODKEY), "Super");
        assert_eq!(format_modifiers(MODKEY | SHIFT), "Super + Shift");
        assert_eq!(
            format_modifiers(MODKEY | CONTROL | SHIFT | MOD1),
            "Super + Ctrl + Shift + Alt"
        );
    }
}
