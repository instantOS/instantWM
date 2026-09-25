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
//! action = { spawn = ["kitty"] }
//!
//! [[keybinds]]
//! modifiers = ["super"]
//! key = "q"
//! action = "none"                    # remove the default binding
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::actions::{KeyAction, NamedAction};
use crate::types::{Key, KeybindOrigin, Keysym, ModMask};

/// A single keybind entry from the TOML config.
///
/// `modifiers` and `key` are deliberately kept as `String` rather than parsed
/// types, and "too broad" cuts two ways here, so both readings are worth
/// stating:
///
/// - As a *type*, `String` is right. These are the config surface, and the
///   unvalidated value is confined to this struct. `parse_modifiers` and
///   [`Keysym::from_name`] convert both fields to checked types before anything
///   else in the codebase sees them, so no unvalidated string escapes.
/// - As a *name set*, breadth is the requirement, not the problem. `key` must
///   accept any XKB keysym name, including the thousands instantWM has no
///   constant for. What was genuinely too broad was the number of *accepted
///   spellings per key*: `enter`, `esc`, `pageup` and `pagedown` each meant two
///   things, so a config could be written in a spelling the tooling never
///   printed. Those aliases are gone, and `Modifier` is a closed enum with one
///   canonical name per bit. Every name the config accepts is a name it also
///   prints, so a binding can be read out of `instantwmctl keybinds` and pasted
///   straight back in.
#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct KeybindSpec {
    #[serde(default)]
    pub modifiers: Vec<String>,
    pub key: String,
    pub action: ActionSpec,
}

/// A configured action: an action name (`"zoom"`, or `"none"` to unbind), a
/// name followed by its arguments (`["set_layout", "grid"]`), or a structured
/// action (`{ set_layout = "grid" }`).
#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(untagged)]
pub enum ActionSpec {
    Named(String),
    WithArgs(Vec<String>),
    Structured(toml::Table),
}

impl ActionSpec {
    /// `"none"` removes a binding instead of running anything.
    fn is_unbind(&self) -> bool {
        matches!(self, Self::Named(name) if name == "none")
    }
}

/// Combine a config's modifier names into a mask.
///
/// Each entry is a canonical [`Modifier`] name, matched case-insensitively.
/// There is no alias table: `super` is the only spelling of the primary
/// modifier, and `mod4` is not accepted for it, because a second spelling per
/// modifier makes the config language ambiguous about which vocabulary it
/// speaks and stops `instantwmctl keybinds` output from round-tripping.
pub fn parse_modifiers(mods: &[String]) -> Result<ModMask, String> {
    mods.iter().try_fold(ModMask::NONE, |mask, name| {
        name.parse::<crate::types::Modifier>()
            .map(|modifier| mask.with(modifier))
            .map_err(|error| error.to_string())
    })
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
        ActionSpec::Structured(table) => compile_structured_action(table),
    }
}

fn compile_structured_action(table: &toml::Table) -> Result<KeyAction, String> {
    if table.len() != 1 {
        return Err("an action table must contain exactly one action".to_string());
    }
    let (name, value) = table.iter().next().expect("table has one entry");

    if name == "sequence" {
        let toml::Value::Array(steps) = value else {
            return Err("'sequence' must be an array of actions".to_string());
        };
        if steps.is_empty() {
            return Err("'sequence' must contain at least one action".to_string());
        }
        return steps
            .iter()
            .map(|step| {
                let action: ActionSpec = step
                    .clone()
                    .try_into()
                    .map_err(|error| format!("invalid action in sequence: {error}"))?;
                compile_action(&action)
            })
            .collect::<Result<_, _>>()
            .map(KeyAction::Sequence);
    }

    let values = match value {
        toml::Value::Array(values) => values.as_slice(),
        value => std::slice::from_ref(value),
    };
    let args = values
        .iter()
        .map(|value| match value {
            toml::Value::String(text) => Ok(text.clone()),
            toml::Value::Integer(number) => Ok(number.to_string()),
            toml::Value::Boolean(boolean) => Ok(boolean.to_string()),
            _ => Err(format!(
                "action '{name}' expects a string, integer, boolean, or array of those values"
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    NamedAction::parse(name, &args).map(KeyAction::Named)
}

fn compile_keybind(spec: &KeybindSpec) -> Result<((ModMask, Keysym), Option<KeyAction>), String> {
    let combo = (
        parse_modifiers(&spec.modifiers)?,
        Keysym::from_name(&spec.key)
            .map_err(|error| error.to_string())?
            .for_binding(),
    );
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
    // A binding's identity is its chord, so the index is keyed on the typed
    // chord rather than a `(u32, u32)` pair that could be assembled backwards.
    let mut index: HashMap<(ModMask, Keysym), usize> = keys
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::keybindings::{CONTROL, MOD1, MOD2, MOD3, MOD5, MODKEY, SHIFT};
    use crate::config::keysyms::*;
    use crate::types::{Keysym, ModMask, Modifier};

    fn parse_keybind(source: &str) -> KeybindSpec {
        #[derive(Deserialize)]
        struct Wrapper {
            keybind: KeybindSpec,
        }

        toml::from_str::<Wrapper>(source).unwrap().keybind
    }

    fn default_key(keysym: Keysym) -> Key {
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
            modifiers = ["alt"]
            key = "p"
            action = "none"
            "#,
        );
        let merged = merge_keybinds(vec![default_key(XK_P)], &[spec], KeybindOrigin::User);
        assert!(merged.is_empty());
    }

    #[test]
    fn uppercase_ascii_keysym_names_use_the_same_binding_as_lowercase() {
        // XKB resolves "A" to lowercase with case-insensitive lookup, but its
        // Unicode and numeric spellings resolve to the uppercase keysym.
        for name in ["A", "U0041", "0x41"] {
            let override_spec = parse_keybind(&format!(
                "[keybind]\nmodifiers = [\"alt\"]\nkey = \"{name}\"\naction = \"toggle_bar\""
            ));
            let merged = merge_keybinds(
                vec![default_key(XK_A)],
                &[override_spec],
                KeybindOrigin::User,
            );
            assert_eq!(merged.len(), 1, "{name} must override the default");
            assert_eq!(merged[0].keysym, XK_A);
            assert!(matches!(
                merged[0].action,
                KeyAction::Named(NamedAction::ToggleBar)
            ));

            let unbind_spec = parse_keybind(&format!(
                "[keybind]\nmodifiers = [\"alt\"]\nkey = \"{name}\"\naction = \"none\""
            ));
            assert!(
                merge_keybinds(vec![default_key(XK_A)], &[unbind_spec], KeybindOrigin::User)
                    .is_empty(),
                "{name} must unbind the default"
            );
        }
    }

    #[test]
    fn merge_keybinds_adds_and_overrides() {
        let specs = [
            parse_keybind(
                r#"
                [keybind]
                modifiers = ["alt"]
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
                "[keybind]\nkey = \"p\"\nmodifiers = [\"alt\"]\naction = \"does_not_exist\"",
            ),
            parse_keybind(
                "[keybind]\nkey = \"p\"\nmodifiers = [\"alt\"]\naction = [\"set_layout\"]",
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
            ("RETURN", XK_RETURN),
            ("page_up", XK_PAGE_UP),
            ("f12", XK_F12),
            ("A", XK_A),
            ("a", XK_A),
            ("7", Keysym::new(XK_0.raw() + 7)),
            ("dead_circumflex", XK_DEAD_CIRCUMFLEX),
            ("XF86AudioMute", XF86XK_AUDIO_MUTE),
        ] {
            assert_eq!(Keysym::from_name(name), Ok(keysym), "{name}");
        }
        assert!(Keysym::from_name("nokey").is_err());
    }

    #[test]
    fn key_aliases_are_rejected_so_the_language_has_one_spelling() {
        // `enter`, `esc`, `pageup` and `pagedown` used to be accepted as
        // aliases for `Return`, `Escape`, `Prior` and `Next`. They resolved to
        // the right keysym, but meant a config could be written in a spelling
        // the tooling never printed, so it could not be read back.
        for alias in ["enter", "esc", "pageup", "pagedown"] {
            assert!(
                Keysym::from_name(alias).is_err(),
                "'{alias}' must not resolve"
            );
        }
        // The canonical spellings still work, including the ambiguous pair the
        // aliases used to paper over: main Enter is `Return`, keypad Enter is
        // `KP_Enter`.
        assert_eq!(Keysym::from_name("Return"), Ok(XK_RETURN));
        assert_eq!(Keysym::from_name("KP_Enter"), Ok(Keysym::new(0xFF8D)));
    }

    #[test]
    fn keysym_names_round_trip_through_their_rendered_form() {
        // Whatever `instantwmctl keybinds` prints has to parse back, or the
        // listing cannot be copied into a config.
        for keysym in [
            XK_A,
            XK_RETURN,
            XK_ESCAPE,
            XK_DEAD_CIRCUMFLEX,
            XK_MINUS,
            XK_SLASH,
            XF86XK_AUDIO_MUTE,
        ] {
            let rendered = keysym.to_config_name();
            assert_eq!(Keysym::from_name(&rendered), Ok(keysym), "{rendered}");
        }
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
            assert_eq!(Keysym::from_name(sym), Ok(keysym));
            assert_eq!(Keysym::from_name(name), Ok(keysym));
            assert_eq!(keysym.to_config_name(), sym);
        }
        assert_eq!(XK_A.to_config_name(), "a");
        assert_eq!(XK_RETURN.to_config_name(), "Return");
        assert_eq!(XK_DEAD_CIRCUMFLEX.to_config_name(), "dead_circumflex");
    }

    #[test]
    fn modifier_names_round_trip_through_their_rendered_mask() {
        for mask in [
            ModMask::NONE,
            MODKEY,
            MODKEY | SHIFT,
            MODKEY | CONTROL | SHIFT | MOD1,
            ModMask::from_modifiers(Modifier::DISPLAY_ORDER),
        ] {
            let rendered = mask.to_string();
            assert_eq!(rendered.parse::<ModMask>().unwrap(), mask, "{rendered}");
        }
        assert_eq!(ModMask::NONE.to_string(), "");
        assert_eq!(MODKEY.to_string(), "Super");
        assert_eq!((MODKEY | SHIFT).to_string(), "Super + Shift");
        assert_eq!(
            (MODKEY | CONTROL | SHIFT | MOD1).to_string(),
            "Super + Control + Shift + Alt"
        );
    }

    #[test]
    fn config_modifier_names_reject_the_removed_aliases() {
        for (spec, expected) in [
            (r#"["super"]"#, MODKEY),
            (r#"["alt"]"#, MOD1),
            (r#"["control"]"#, CONTROL),
            (r#"["shift"]"#, SHIFT),
        ] {
            let merged = merge_keybinds(
                Vec::new(),
                &[parse_keybind(&format!(
                    "[keybind]\nmodifiers = {spec}\nkey = \"p\"\naction = \"zoom\""
                ))],
                KeybindOrigin::User,
            );
            assert_eq!(merged.len(), 1, "{spec}");
            assert_eq!(merged[0].mod_mask, expected, "{spec}");
        }

        // `mod`, `mod4` and `modkey` all used to mean Super, and `mod1` used to
        // mean Alt. One spelling per modifier, or a config cannot be read back
        // as the form the tooling prints.
        for alias in ["mod", "mod4", "modkey", "mod1", "ctrl"] {
            let merged = merge_keybinds(
                Vec::new(),
                &[parse_keybind(&format!(
                    "[keybind]\nmodifiers = [\"{alias}\"]\nkey = \"p\"\naction = \"zoom\""
                ))],
                KeybindOrigin::User,
            );
            assert!(merged.is_empty(), "'{alias}' must be rejected");
        }
    }

    #[test]
    fn positional_modifier_names_still_work_where_meaning_is_universal() {
        // Mod2/Mod3/Mod5 have no universal meaning, so they are named by
        // position and must keep working for AltGr and level-3 setups.
        for (name, expected) in [("mod2", MOD2), ("mod3", MOD3), ("mod5", MOD5)] {
            let merged = merge_keybinds(
                Vec::new(),
                &[parse_keybind(&format!(
                    "[keybind]\nmodifiers = [\"{name}\"]\nkey = \"p\"\naction = \"zoom\""
                ))],
                KeybindOrigin::User,
            );
            assert_eq!(merged.len(), 1, "{name}");
            assert_eq!(merged[0].mod_mask, expected, "{name}");
        }
    }
}
