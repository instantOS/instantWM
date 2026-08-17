//! TOML-configurable keybindings.
//!
//! Parses `[[keybinds]]` and `[[desktop_keybinds]]` entries from the config
//! file and merges them with the compiled defaults. TOML entries override
//! defaults where `(mod_mask, keysym)` matches; unmatched entries are appended.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::actions::{ActionMeta, KeyAction, NamedAction, get_action_metadata, parse_named_action};
use crate::config::keybindings::{CONTROL, MOD1, MODKEY, SHIFT};
use crate::config::keysyms::*;
use crate::types::{Key, KeybindOrigin};

/// A single keybind entry from the TOML config.
#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct KeybindSpec {
    #[serde(default)]
    pub modifiers: Vec<String>,
    pub key: String,
    pub action: ActionSpec,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(untagged)]
pub enum ActionSpec {
    Named(String),
    Structured(StructuredAction),
}

#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredAction {
    /// Execute each action in order.
    Sequence(Vec<ActionSpec>),
    Spawn(Vec<String>),
    Unbind(bool),
    None,
    SetLayout(String),
    FocusStack(String),
    IncMasterCount(i32),
    KeyboardLayout(String),
    SetMode(String),
}

pub fn parse_modifiers(mods: &[String]) -> Option<u32> {
    let mut mask = 0u32;
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "super" | "mod" | "mod4" | "modkey" => mask |= MODKEY,
            "shift" => mask |= SHIFT,
            "control" | "ctrl" => mask |= CONTROL,
            "alt" | "mod1" => mask |= MOD1,
            "" => {}
            other => {
                eprintln!("instantwm: unknown modifier '{other}' in keybind config");
                return None;
            }
        }
    }
    Some(mask)
}

pub fn parse_keysym(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    if lower.len() == 1 {
        let ch = lower.chars().next().unwrap();
        match ch {
            'a'..='z' => return Some(XK_A + (ch as u32 - 'a' as u32)),
            '0'..='9' => return Some(XK_0 + (ch as u32 - '0' as u32)),
            '-' => return Some(XK_MINUS),
            '+' => return Some(XK_PLUS),
            ',' => return Some(XK_COMMA),
            '.' => return Some(XK_PERIOD),
            '/' => return Some(XK_SLASH),
            ';' => return Some(XK_SEMICOLON),
            ':' => return Some(XK_COLON),
            '=' => return Some(XK_EQUAL),
            '[' => return Some(XK_BRACKET_LEFT),
            ']' => return Some(XK_BRACKET_RIGHT),
            '\\' => return Some(XK_BACKSLASH),
            '`' => return Some(XK_GRAVE),
            '\'' => return Some(XK_APOSTROPHE),
            _ => {}
        }
    }

    match lower.as_str() {
        "return" | "enter" => Some(XK_RETURN),
        "backspace" => Some(XK_BACKSPACE),
        "tab" => Some(XK_TAB),
        "escape" | "esc" => Some(XK_ESCAPE),
        "delete" => Some(XK_DELETE),
        "home" => Some(XK_HOME),
        "end" => Some(XK_END),
        "insert" => Some(XK_INSERT),
        "left" => Some(XK_LEFT),
        "up" => Some(XK_UP),
        "right" => Some(XK_RIGHT),
        "down" => Some(XK_DOWN),
        "page_up" | "pageup" | "prior" => Some(XK_PAGE_UP),
        "page_down" | "pagedown" | "next" => Some(XK_PAGE_DOWN),
        "f1" => Some(XK_F1),
        "f2" => Some(XK_F2),
        "f3" => Some(XK_F3),
        "f4" => Some(XK_F4),
        "f5" => Some(XK_F5),
        "f6" => Some(XK_F6),
        "f7" => Some(XK_F7),
        "f8" => Some(XK_F8),
        "f9" => Some(XK_F9),
        "f10" => Some(XK_F10),
        "f11" => Some(XK_F11),
        "f12" => Some(XK_F12),
        "space" => Some(XK_SPACE),
        "minus" => Some(XK_MINUS),
        "plus" => Some(XK_PLUS),
        "comma" => Some(XK_COMMA),
        "period" | "dot" => Some(XK_PERIOD),
        "slash" => Some(XK_SLASH),
        "semicolon" => Some(XK_SEMICOLON),
        "colon" => Some(XK_COLON),
        "equal" | "equals" => Some(XK_EQUAL),
        "bracket_left" | "bracketleft" => Some(XK_BRACKET_LEFT),
        "bracket_right" | "bracketright" => Some(XK_BRACKET_RIGHT),
        "backslash" => Some(XK_BACKSLASH),
        "grave" | "backtick" => Some(XK_GRAVE),
        "apostrophe" => Some(XK_APOSTROPHE),
        "print" | "printscreen" => Some(XK_PRINT),
        "dead_circumflex" => Some(XK_DEAD_CIRCUMFLEX),
        "xf86monbrightnessup" | "brightnessup" => Some(XF86XK_MON_BRIGHTNESS_UP),
        "xf86monbrightnessdown" | "brightnessdown" => Some(XF86XK_MON_BRIGHTNESS_DOWN),
        "xf86audiolowervolume" | "volumedown" => Some(XF86XK_AUDIO_LOWER_VOLUME),
        "xf86audiomute" | "volumemute" | "mute" => Some(XF86XK_AUDIO_MUTE),
        "xf86audiomicmute" | "micmute" => Some(XF86XK_AUDIO_MIC_MUTE),
        "xf86audioraisevolume" | "volumeup" => Some(XF86XK_AUDIO_RAISE_VOLUME),
        "xf86audioplay" | "audioplay" => Some(XF86XK_AUDIO_PLAY),
        "xf86audiopause" | "audiopause" => Some(XF86XK_AUDIO_PAUSE),
        "xf86audionext" | "audionext" => Some(XF86XK_AUDIO_NEXT),
        "xf86audioprev" | "audioprev" => Some(XF86XK_AUDIO_PREV),
        _ => {
            eprintln!("instantwm: unknown key name '{name}' in keybind config");
            None
        }
    }
}

pub fn get_all_actions() -> Vec<ActionMeta> {
    let mut actions = get_action_metadata();
    actions.sort_by(|a, b| a.name.cmp(b.name));
    actions
}

pub fn get_actions_for_ipc() -> Vec<crate::ipc_types::ActionInfo> {
    get_all_actions()
        .into_iter()
        .map(|a| crate::ipc_types::ActionInfo {
            name: a.name.to_string(),
            description: Some(a.doc.to_string()),
            arg_example: a.arg_example.map(|s| s.to_string()),
        })
        .collect()
}

pub fn format_action_list_text(actions: &[crate::ipc_types::ActionInfo]) -> String {
    let mut output = String::new();
    let max_name_len = actions.iter().map(|a| a.name.len()).max().unwrap_or(0);
    let name_width = max_name_len.max(8);

    use std::fmt::Write;
    writeln!(
        output,
        "{:<width$} | {:<20} | DESCRIPTION",
        "ACTION",
        "ARGUMENTS",
        width = name_width
    )
    .unwrap();
    writeln!(
        output,
        "{:-<width$}-|-{:-<20}-|-{:-<30}",
        "",
        "",
        "",
        width = name_width
    )
    .unwrap();

    for action in actions {
        let args = action.arg_example.as_deref().unwrap_or("-");
        let desc = action.description.as_deref().unwrap_or("");
        writeln!(
            output,
            "{:<width$} | {:<20} | {}",
            action.name,
            args,
            desc,
            width = name_width
        )
        .unwrap();
    }
    output
}

pub fn print_actions(json: bool) {
    let actions: Vec<crate::ipc_types::ActionInfo> = get_all_actions()
        .into_iter()
        .map(|a| crate::ipc_types::ActionInfo {
            name: a.name.to_string(),
            description: Some(a.doc.to_string()),
            arg_example: a.arg_example.map(|s| s.to_string()),
        })
        .collect();

    if json {
        if let Ok(output) = serde_json::to_string_pretty(&actions) {
            println!("{}", output);
        } else {
            eprintln!("Error generating JSON");
        }
        return;
    }

    print!("{}", format_action_list_text(&actions));
}

pub fn compile_named_action(name: &str) -> Option<KeyAction> {
    let action = parse_named_action(name)?;
    Some(KeyAction::Named {
        action,
        args: Vec::new(),
    })
}

fn compile_action(spec: &ActionSpec) -> Option<KeyAction> {
    match spec {
        ActionSpec::Structured(StructuredAction::Unbind(_)) => None,
        ActionSpec::Structured(StructuredAction::None) => None,
        ActionSpec::Structured(StructuredAction::Sequence(specs)) => {
            let actions = specs
                .iter()
                .map(compile_action)
                .collect::<Option<Vec<_>>>()?;
            if actions.is_empty() {
                None
            } else {
                Some(KeyAction::Sequence(actions))
            }
        }
        ActionSpec::Structured(StructuredAction::Spawn(argv)) => Some(KeyAction::Named {
            action: NamedAction::Spawn,
            args: argv.clone(),
        }),
        ActionSpec::Structured(StructuredAction::SetLayout(name)) => Some(KeyAction::Named {
            action: NamedAction::SetLayout,
            args: vec![name.clone()],
        }),
        ActionSpec::Structured(StructuredAction::FocusStack(dir)) => Some(KeyAction::Named {
            action: NamedAction::FocusStack,
            args: vec![dir.clone()],
        }),
        ActionSpec::Structured(StructuredAction::IncMasterCount(n)) => Some(KeyAction::Named {
            action: NamedAction::IncMasterCount,
            args: vec![n.to_string()],
        }),
        ActionSpec::Structured(StructuredAction::KeyboardLayout(name)) => Some(KeyAction::Named {
            action: NamedAction::KeyboardLayout,
            args: vec![name.clone()],
        }),
        ActionSpec::Structured(StructuredAction::SetMode(name)) => Some(KeyAction::Named {
            action: NamedAction::SetMode,
            args: vec![name.clone()],
        }),
        ActionSpec::Named(name) => compile_named_action(name),
    }
}

pub fn merge_keybinds(
    defaults: Vec<Key>,
    specs: &[KeybindSpec],
    origin: KeybindOrigin,
) -> Vec<Key> {
    let mut keys: Vec<Option<Key>> = defaults.into_iter().map(Some).collect();
    let mut index: HashMap<(u32, u32), usize> = HashMap::new();
    for (i, k) in keys.iter().enumerate() {
        if let Some(k) = k {
            index.insert((k.mod_mask, k.keysym), i);
        }
    }

    for spec in specs {
        let mod_mask = match parse_modifiers(&spec.modifiers) {
            Some(m) => m,
            None => continue,
        };
        let keysym = match parse_keysym(&spec.key) {
            Some(k) => k,
            None => continue,
        };

        let combo = (mod_mask, keysym);

        match &spec.action {
            ActionSpec::Structured(StructuredAction::Unbind(true))
            | ActionSpec::Structured(StructuredAction::None) => {
                if let Some(&idx) = index.get(&combo) {
                    keys[idx] = None;
                    index.remove(&combo);
                }
            }
            ActionSpec::Named(name) if name.eq_ignore_ascii_case("none") => {
                if let Some(&idx) = index.get(&combo) {
                    keys[idx] = None;
                    index.remove(&combo);
                }
            }
            _ => {
                if let Some(action) = compile_action(&spec.action) {
                    let new_key = Key {
                        mod_mask,
                        keysym,
                        action,
                        origin,
                    };
                    if let Some(&idx) = index.get(&combo) {
                        keys[idx] = Some(new_key);
                    } else {
                        let idx = keys.len();
                        keys.push(Some(new_key));
                        index.insert(combo, idx);
                    }
                }
            }
        }
    }

    keys.into_iter().flatten().collect()
}

// ---------------------------------------------------------------------------
// Reverse maps: render a (mod_mask, keysym) pair as the user would type it.
//
// These are the inverses of `parse_modifiers` / `parse_keysym`, used by
// `instantwmctl keybinds` to show chords readably. They deliberately mirror the
// accepted input names so what the user types in config.toml is what they see
// back.
// ---------------------------------------------------------------------------

/// Render a modifier mask as `Super + Ctrl + Shift` (empty when no modifiers).
pub fn format_modifiers(mask: u32) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if mask & MODKEY != 0 {
        parts.push("Super");
    }
    if mask & CONTROL != 0 {
        parts.push("Ctrl");
    }
    if mask & SHIFT != 0 {
        parts.push("Shift");
    }
    if mask & MOD1 != 0 {
        parts.push("Alt");
    }
    parts.join(" + ")
}

/// Render a keysym as a short, human-friendly key name.
pub fn format_keysym(keysym: u32) -> String {
    // Letter keysyms are lowercase-only (`XK_A` is 0x61), so the printable
    // ASCII branches below cover `a`-`z` and `0`-`9`; there is no uppercase
    // keysym range.
    if (b'a' as u32..=b'z' as u32).contains(&keysym) {
        return (keysym as u8 as char).to_string();
    }
    if (b'0' as u32..=b'9' as u32).contains(&keysym) {
        return (keysym as u8 as char).to_string();
    }

    let name = match keysym {
        XK_RETURN => "Return",
        XK_BACKSPACE => "Backspace",
        XK_TAB => "Tab",
        XK_ESCAPE => "Esc",
        XK_DELETE => "Delete",
        XK_HOME => "Home",
        XK_END => "End",
        XK_INSERT => "Insert",
        XK_LEFT => "Left",
        XK_UP => "Up",
        XK_RIGHT => "Right",
        XK_DOWN => "Down",
        XK_PAGE_UP => "PageUp",
        XK_PAGE_DOWN => "PageDown",
        XK_SPACE => "Space",
        XK_MINUS => "-",
        XK_PLUS => "+",
        XK_COMMA => ",",
        XK_PERIOD => ".",
        XK_SLASH => "/",
        XK_SEMICOLON => ";",
        XK_COLON => ":",
        XK_EQUAL => "=",
        XK_BRACKET_LEFT => "[",
        XK_BRACKET_RIGHT => "]",
        XK_BACKSLASH => "\\",
        XK_GRAVE => "`",
        XK_APOSTROPHE => "'",
        XK_DEAD_CIRCUMFLEX => "DeadCircumflex",
        XK_PRINT => "Print",
        XK_F1 => "F1",
        XK_F2 => "F2",
        XK_F3 => "F3",
        XK_F4 => "F4",
        XK_F5 => "F5",
        XK_F6 => "F6",
        XK_F7 => "F7",
        XK_F8 => "F8",
        XK_F9 => "F9",
        XK_F10 => "F10",
        XK_F11 => "F11",
        XK_F12 => "F12",
        XF86XK_MON_BRIGHTNESS_UP => "BrightnessUp",
        XF86XK_MON_BRIGHTNESS_DOWN => "BrightnessDown",
        XF86XK_AUDIO_LOWER_VOLUME => "VolumeDown",
        XF86XK_AUDIO_MUTE => "Mute",
        XF86XK_AUDIO_MIC_MUTE => "MicMute",
        XF86XK_AUDIO_RAISE_VOLUME => "VolumeUp",
        XF86XK_AUDIO_PLAY => "AudioPlay",
        XF86XK_AUDIO_PAUSE => "AudioPause",
        XF86XK_AUDIO_NEXT => "AudioNext",
        XF86XK_AUDIO_PREV => "AudioPrev",
        _ => return format!("0x{keysym:04X}"),
    };
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_keybind(source: &str) -> KeybindSpec {
        #[derive(Deserialize)]
        struct Wrapper {
            keybind: KeybindSpec,
        }

        toml::from_str::<Wrapper>(source).unwrap().keybind
    }

    #[test]
    fn test_merge_keybinds_none_action_removes_default() {
        let defaults = vec![Key {
            mod_mask: MOD1,
            keysym: XK_P,
            action: KeyAction::Named {
                action: NamedAction::None,
                args: Vec::new(),
            },
            origin: KeybindOrigin::CompiledDefault,
        }];

        let specs = vec![KeybindSpec {
            modifiers: vec!["Mod1".to_string()],
            key: "p".to_string(),
            action: ActionSpec::Named("none".to_string()),
        }];

        let merged = merge_keybinds(defaults, &specs, KeybindOrigin::User);
        assert_eq!(merged.len(), 0);
    }

    #[test]
    fn test_merge_keybinds_structured_none_removes_default() {
        let defaults = vec![Key {
            mod_mask: MOD1,
            keysym: XK_P,
            action: KeyAction::Named {
                action: NamedAction::None,
                args: Vec::new(),
            },
            origin: KeybindOrigin::CompiledDefault,
        }];

        let specs = vec![KeybindSpec {
            modifiers: vec!["Mod1".to_string()],
            key: "p".to_string(),
            action: ActionSpec::Structured(StructuredAction::None),
        }];

        let merged = merge_keybinds(defaults, &specs, KeybindOrigin::User);
        assert_eq!(merged.len(), 0);
    }

    #[test]
    fn test_merge_keybinds_adds_and_overrides() {
        let defaults = vec![Key {
            mod_mask: MOD1,
            keysym: XK_P,
            action: KeyAction::Named {
                action: NamedAction::None,
                args: Vec::new(),
            },
            origin: KeybindOrigin::CompiledDefault,
        }];

        let specs = vec![
            KeybindSpec {
                modifiers: vec!["Mod1".to_string()],
                key: "p".to_string(),
                action: ActionSpec::Named("toggle_bar".to_string()),
            },
            KeybindSpec {
                modifiers: vec!["Mod1".to_string()],
                key: "o".to_string(),
                action: ActionSpec::Named("focus_left".to_string()),
            },
        ];

        let merged = merge_keybinds(defaults, &specs, KeybindOrigin::User);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].keysym, XK_P);
        assert_eq!(merged[1].keysym, XK_O);
        assert!(merged.iter().all(|key| key.origin == KeybindOrigin::User));
    }

    #[test]
    fn sequence_action_parses_and_compiles_in_order() {
        let spec = parse_keybind(
            r#"
            [keybind]
            modifiers = []
            key = "f"
            action = { sequence = [
                { set_mode = "default" },
                { spawn = ["ins", "assist", "run", "sf"] },
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
        assert_eq!(actions.len(), 2);
        assert!(matches!(
            &actions[0],
            KeyAction::Named { action: NamedAction::SetMode, args }
                if args == &["default".to_string()]
        ));
        assert!(matches!(
            &actions[1],
            KeyAction::Named { action: NamedAction::Spawn, args }
                if args == &[
                    "ins".to_string(),
                    "assist".to_string(),
                    "run".to_string(),
                    "sf".to_string(),
                ]
        ));
    }

    #[test]
    fn sequence_action_rejects_empty_or_non_executable_members() {
        let empty = parse_keybind(
            r#"
            [keybind]
            modifiers = []
            key = "f"
            action = { sequence = [] }
            "#,
        );
        let containing_none = parse_keybind(
            r#"
            [keybind]
            modifiers = []
            key = "f"
            action = { sequence = [{ set_mode = "default" }, { unbind = true }] }
            "#,
        );

        assert!(merge_keybinds(Vec::new(), &[empty], KeybindOrigin::User).is_empty());
        assert!(merge_keybinds(Vec::new(), &[containing_none], KeybindOrigin::User).is_empty());
    }

    #[test]
    fn nested_sequences_are_supported() {
        let spec = parse_keybind(
            r#"
            [keybind]
            modifiers = []
            key = "f"
            action = { sequence = [
                { set_mode = "default" },
                { sequence = [
                    { spawn = ["first"] },
                    { spawn = ["second"] },
                ] },
            ] }
            "#,
        );

        let merged = merge_keybinds(Vec::new(), &[spec], KeybindOrigin::User);
        let KeyAction::Sequence(actions) = &merged[0].action else {
            panic!("expected a sequence action");
        };
        assert!(matches!(actions.get(1), Some(KeyAction::Sequence(inner)) if inner.len() == 2));
    }

    #[test]
    fn supported_dead_circumflex_keysym_formats_by_name() {
        let keysym = parse_keysym("dead_circumflex").expect("supported keysym");

        assert_eq!(format_keysym(keysym), "DeadCircumflex");
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

    #[test]
    fn punctuation_keysyms_parse_and_format_bidirectionally() {
        let symbols = [
            ("-", "minus", XK_MINUS),
            ("+", "plus", XK_PLUS),
            (",", "comma", XK_COMMA),
            (".", "period", XK_PERIOD),
            ("/", "slash", XK_SLASH),
            (";", "semicolon", XK_SEMICOLON),
            (":", "colon", XK_COLON),
            ("=", "equal", XK_EQUAL),
            ("[", "bracket_left", XK_BRACKET_LEFT),
            ("]", "bracket_right", XK_BRACKET_RIGHT),
            ("\\", "backslash", XK_BACKSLASH),
            ("`", "grave", XK_GRAVE),
            ("'", "apostrophe", XK_APOSTROPHE),
        ];

        for (sym, name, expected_keysym) in symbols {
            assert_eq!(parse_keysym(sym), Some(expected_keysym));
            assert_eq!(parse_keysym(name), Some(expected_keysym));
            assert_eq!(format_keysym(expected_keysym), sym);
        }
    }
}
