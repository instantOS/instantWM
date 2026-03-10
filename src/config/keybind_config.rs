//! TOML-configurable keybindings.
//!
//! Parses `[[keybinds]]` and `[[desktop_keybinds]]` entries from the config
//! file and merges them with the compiled defaults. TOML entries override
//! defaults where `(mod_mask, keysym)` matches; unmatched entries are appended.

use std::collections::HashMap;
use std::rc::Rc;

use serde::Deserialize;
use serde::Serialize;

use crate::config::keybindings::{CONTROL, MOD1, MODKEY, SHIFT};
use crate::config::keysyms::*;
use crate::types::Key;

// ---------------------------------------------------------------------------
// TOML deserialization types
// ---------------------------------------------------------------------------

/// A single keybind entry from the TOML config.
#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct KeybindSpec {
    /// Modifier keys, e.g. `["Super", "Shift"]`.
    #[serde(default)]
    pub modifiers: Vec<String>,
    /// Key name, e.g. `"Return"`, `"j"`, `"F1"`, `"space"`.
    pub key: String,
    /// Action to perform.
    pub action: ActionSpec,
}

/// An action that a keybind can trigger.
#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(untagged)]
pub enum ActionSpec {
    /// A named WM action: `"zoom"`, `"focus_next"`, etc.
    Named(String),
    /// A structured action: `{ spawn = [...] }` or `{ unbind = true }`.
    Structured(StructuredAction),
}

/// Structured action variants parsed from inline TOML tables.
#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredAction {
    /// Spawn an external command: `{ spawn = ["kitty", "--arg"] }`.
    Spawn(Vec<String>),
    /// Remove the default binding for this key combo: `{ unbind = true }`.
    Unbind(bool),
    /// Set layout: `{ set_layout = "tile" }`.
    SetLayout(String),
    /// Focus stack direction: `{ focus_stack = "next" }`.
    FocusStack(String),
    /// Set mfact delta: `{ set_mfact = 0.05 }`.
    SetMfact(f64),
    /// Increment nmaster: `{ inc_nmaster = 1 }`.
    IncNmaster(i32),
    /// Set keyboard layout by name: `{ keyboard_layout = "de" }`.
    KeyboardLayout(String),
}

// ---------------------------------------------------------------------------
// Modifier parsing
// ---------------------------------------------------------------------------

/// Parse a list of modifier name strings into a combined bitmask.
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

// ---------------------------------------------------------------------------
// Keysym parsing
// ---------------------------------------------------------------------------

/// Parse a key name string into an X11 keysym value.
pub fn parse_keysym(name: &str) -> Option<u32> {
    // Normalize: case-insensitive matching for named keys,
    // single lowercase letter for letter keys.
    let lower = name.to_ascii_lowercase();

    // Single character keys
    if lower.len() == 1 {
        let ch = lower.chars().next().unwrap();
        return match ch {
            'a'..='z' => Some(XK_A + (ch as u32 - 'a' as u32)),
            '0'..='9' => Some(XK_0 + (ch as u32 - '0' as u32)),
            _ => None,
        };
    }

    match lower.as_str() {
        // Control / navigation
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

        // Function keys
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

        // Whitespace / punctuation
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

        // XF86 media keys
        "xf86monbrightnessup" | "brightnessup" => Some(XF86XK_MON_BRIGHTNESS_UP),
        "xf86monbrightnessdown" | "brightnessdown" => Some(XF86XK_MON_BRIGHTNESS_DOWN),
        "xf86audiolowervolume" | "volumedown" => Some(XF86XK_AUDIO_LOWER_VOLUME),
        "xf86audiomute" | "volumemute" | "mute" => Some(XF86XK_AUDIO_MUTE),
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

// ---------------------------------------------------------------------------
// Action compilation
// ---------------------------------------------------------------------------

use crate::animation;
use crate::client::{kill_client, shut_kill, toggle_fake_fullscreen_x11, zoom};
use crate::contexts::WmCtx;
use crate::floating::{center_window, distribute_clients, key_resize, toggle_maximized};
use crate::focus::{direction_focus, focus_last_client, focus_stack};
use crate::keyboard::{down_key, up_key};
use crate::layouts::{
    cycle_layout_direction, inc_nmaster_by, set_layout, set_mfact, toggle_layout, LayoutKind,
};
use crate::monitor::{focus_monitor, move_to_monitor_and_follow};
use crate::mouse::{begin_keyboard_move, draw_window};
use crate::overlay::{create_overlay, set_overlay};
use crate::push::{push, Direction as PushDirection};
use crate::scratchpad::{scratchpad_make, scratchpad_toggle};
use crate::tags::{
    follow_view, last_view, move_client, quit, shift_tag, shift_view, toggle_fullscreen_overview,
    toggle_overview, win_view,
};
use crate::toggles::toggle_bar;
use crate::toggles::{
    toggle_alt_tag, toggle_animated, toggle_double_draw, toggle_prefix, toggle_show_tags,
    toggle_sticky, unhide_all,
};
use crate::types::{Direction, MonitorDirection, StackDirection, TagMask, ToggleAction};
use crate::util::spawn;

/// Compile an [`ActionSpec`] into an `Rc<dyn Fn(&mut WmCtx)>` closure.
///
/// Returns `None` for `Unbind` (caller should remove the binding) or
/// unrecognized action names (with a warning printed).
fn compile_action(spec: &ActionSpec) -> Option<Rc<dyn Fn(&mut WmCtx)>> {
    match spec {
        ActionSpec::Structured(StructuredAction::Unbind(_)) => None,

        ActionSpec::Structured(StructuredAction::Spawn(argv)) => {
            let argv = argv.clone();
            Some(Rc::new(move |ctx| spawn(ctx, &argv)))
        }

        ActionSpec::Structured(StructuredAction::SetLayout(name)) => {
            let kind = match name.to_ascii_lowercase().as_str() {
                "tile" | "tiling" => LayoutKind::Tile,
                "float" | "floating" => LayoutKind::Floating,
                "monocle" => LayoutKind::Monocle,
                "grid" => LayoutKind::Grid,
                other => {
                    eprintln!("instantwm: unknown layout '{other}' in keybind config");
                    return None;
                }
            };
            Some(Rc::new(move |ctx| set_layout(ctx, kind)))
        }

        ActionSpec::Structured(StructuredAction::FocusStack(dir)) => {
            let direction = match dir.to_ascii_lowercase().as_str() {
                "next" | "down" | "forward" => StackDirection::Next,
                "prev" | "previous" | "up" | "backward" => StackDirection::Previous,
                other => {
                    eprintln!("instantwm: unknown focus_stack direction '{other}'");
                    return None;
                }
            };
            Some(Rc::new(move |ctx| focus_stack(ctx, direction)))
        }

        ActionSpec::Structured(StructuredAction::SetMfact(delta)) => {
            let d = *delta as f32;
            Some(Rc::new(move |ctx| set_mfact(ctx, d)))
        }

        ActionSpec::Structured(StructuredAction::IncNmaster(n)) => {
            let n = *n;
            Some(Rc::new(move |ctx| inc_nmaster_by(ctx, n)))
        }

        ActionSpec::Structured(StructuredAction::KeyboardLayout(name)) => {
            let name = name.clone();
            Some(Rc::new(move |ctx| {
                crate::keyboard_layout::set_keyboard_layout_by_name(ctx, &name);
            }))
        }

        ActionSpec::Named(name) => compile_named_action(name),
    }
}

/// Compile a named action string into a closure.
fn compile_named_action(name: &str) -> Option<Rc<dyn Fn(&mut WmCtx)>> {
    let action: Rc<dyn Fn(&mut WmCtx)> = match name.to_ascii_lowercase().as_str() {
        // Client operations
        "zoom" => Rc::new(zoom),
        "kill" | "kill_client" => Rc::new(|ctx| {
            if let Some(win) = ctx.selected_client() {
                kill_client(ctx, win)
            }
        }),
        "shut_kill" => Rc::new(|ctx| shut_kill(ctx)),
        "quit" => Rc::new(|_| quit()),

        // Focus
        "focus_next" => Rc::new(|ctx| focus_stack(ctx, StackDirection::Next)),
        "focus_prev" | "focus_previous" => {
            Rc::new(|ctx| focus_stack(ctx, StackDirection::Previous))
        }
        "focus_last" | "focus_last_client" => Rc::new(|ctx| focus_last_client(ctx)),
        "focus_up" => Rc::new(|ctx| direction_focus(ctx, Direction::Up)),
        "focus_down" => Rc::new(|ctx| direction_focus(ctx, Direction::Down)),
        "focus_left" => Rc::new(|ctx| direction_focus(ctx, Direction::Left)),
        "focus_right" => Rc::new(|ctx| direction_focus(ctx, Direction::Right)),
        "down_key" => Rc::new(|ctx| down_key(ctx, StackDirection::Next)),
        "up_key" => Rc::new(|ctx| up_key(ctx, StackDirection::Previous)),

        // Layout
        "toggle_layout" => Rc::new(toggle_layout),
        "layout_tile" => Rc::new(|ctx| set_layout(ctx, LayoutKind::Tile)),
        "layout_float" | "layout_floating" => Rc::new(|ctx| set_layout(ctx, LayoutKind::Floating)),
        "layout_monocle" => Rc::new(|ctx| set_layout(ctx, LayoutKind::Monocle)),
        "layout_grid" => Rc::new(|ctx| set_layout(ctx, LayoutKind::Grid)),
        "cycle_layout_next" => Rc::new(|ctx| cycle_layout_direction(ctx, true)),
        "cycle_layout_prev" => Rc::new(|ctx| cycle_layout_direction(ctx, false)),
        "inc_nmaster" => Rc::new(|ctx| inc_nmaster_by(ctx, 1)),
        "dec_nmaster" => Rc::new(|ctx| inc_nmaster_by(ctx, -1)),
        "mfact_grow" => Rc::new(|ctx| set_mfact(ctx, 0.05)),
        "mfact_shrink" => Rc::new(|ctx| set_mfact(ctx, -0.05)),

        // Floating
        "center_window" => Rc::new(|ctx| {
            if let Some(win) = ctx.selected_client() {
                center_window(ctx, win)
            }
        }),
        "toggle_maximized" => Rc::new(toggle_maximized),
        "distribute_clients" => Rc::new(distribute_clients),

        // Resize (floating)
        "key_resize_up" => Rc::new(|ctx| {
            if let Some(win) = ctx.selected_client() {
                key_resize(ctx, win, Direction::Up)
            }
        }),
        "key_resize_down" => Rc::new(|ctx| {
            if let Some(win) = ctx.selected_client() {
                key_resize(ctx, win, Direction::Down)
            }
        }),
        "key_resize_left" => Rc::new(|ctx| {
            if let Some(win) = ctx.selected_client() {
                key_resize(ctx, win, Direction::Left)
            }
        }),
        "key_resize_right" => Rc::new(|ctx| {
            if let Some(win) = ctx.selected_client() {
                key_resize(ctx, win, Direction::Right)
            }
        }),

        // Push (reorder in stack)
        "push_up" => Rc::new(|ctx| {
            if let Some(win) = ctx.selected_client() {
                push(ctx, win, PushDirection::Up)
            }
        }),
        "push_down" => Rc::new(|ctx| {
            if let Some(win) = ctx.selected_client() {
                push(ctx, win, PushDirection::Down)
            }
        }),

        // Tags / views
        "last_view" => Rc::new(|ctx| last_view(ctx)),
        "follow_view" => Rc::new(|ctx| follow_view(ctx)),
        "win_view" => Rc::new(|ctx| win_view(ctx)),
        "scroll_left" => Rc::new(|ctx| animation::anim_scroll(ctx, Direction::Left)),
        "scroll_right" => Rc::new(|ctx| animation::anim_scroll(ctx, Direction::Right)),
        "move_client_left" => Rc::new(|ctx| move_client(ctx, Direction::Left)),
        "move_client_right" => Rc::new(|ctx| move_client(ctx, Direction::Right)),
        "shift_tag_left" => Rc::new(|ctx| shift_tag(ctx, Direction::Left, 1)),
        "shift_tag_right" => Rc::new(|ctx| shift_tag(ctx, Direction::Right, 1)),
        "shift_view_left" => Rc::new(|ctx| shift_view(ctx, Direction::Left)),
        "shift_view_right" => Rc::new(|ctx| shift_view(ctx, Direction::Right)),
        "view_all" => Rc::new(|ctx| crate::tags::view::view(ctx, TagMask::ALL_BITS)),
        "tag_all" => Rc::new(|ctx| {
            if let Some(win) = ctx.selected_client() {
                crate::tags::client_tags::set_client_tag_ctx(ctx, win, TagMask::ALL_BITS)
            }
        }),
        "toggle_overview" => Rc::new(|ctx| toggle_overview(ctx, TagMask::ALL_BITS)),
        "toggle_fullscreen_overview" => {
            Rc::new(|ctx| toggle_fullscreen_overview(ctx, TagMask::ALL_BITS))
        }

        // Monitor
        "focus_mon_prev" => Rc::new(|ctx| focus_monitor(ctx, MonitorDirection::PREV)),
        "focus_mon_next" => Rc::new(|ctx| focus_monitor(ctx, MonitorDirection::NEXT)),
        "follow_mon_prev" => Rc::new(|ctx| move_to_monitor_and_follow(ctx, MonitorDirection::PREV)),
        "follow_mon_next" => Rc::new(|ctx| move_to_monitor_and_follow(ctx, MonitorDirection::NEXT)),

        // Overlay
        "set_overlay" => Rc::new(set_overlay),
        "create_overlay" => Rc::new(|ctx| {
            if let Some(win) = ctx.selected_client() {
                create_overlay(ctx, win)
            }
        }),

        // Scratchpad
        "scratchpad_toggle" => Rc::new(|ctx| scratchpad_toggle(ctx, None)),
        "scratchpad_make" => Rc::new(|ctx| scratchpad_make(ctx, None)),

        // Bar
        "toggle_bar" => Rc::new(|ctx| toggle_bar(ctx)),

        // Toggles
        "toggle_sticky" => Rc::new(|ctx| {
            if let Some(win) = ctx.selected_client() {
                toggle_sticky(ctx.core_mut(), win)
            }
        }),
        "toggle_alt_tag" => Rc::new(|ctx| toggle_alt_tag(ctx, ToggleAction::Toggle)),
        "toggle_animated" => Rc::new(|ctx| toggle_animated(ctx.core_mut(), ToggleAction::Toggle)),
        "toggle_show_tags" => Rc::new(|ctx| toggle_show_tags(ctx, ToggleAction::Toggle)),
        "toggle_double_draw" => Rc::new(|ctx| toggle_double_draw(ctx.core_mut())),
        "toggle_prefix" => Rc::new(|ctx| toggle_prefix(ctx)),
        "unhide_all" => Rc::new(|ctx| unhide_all(ctx)),
        "hide" => Rc::new(|ctx| {
            if let Some(win) = ctx.selected_client() {
                crate::client::hide(ctx, win)
            }
        }),

        // Fake fullscreen (X11)
        "toggle_fake_fullscreen" => Rc::new(|ctx| {
            if let crate::contexts::WmCtx::X11(ref mut ctx_x11) = ctx {
                toggle_fake_fullscreen_x11(ctx_x11)
            }
        }),

        // Mouse-driven operations
        "draw_window" => Rc::new(draw_window),
        "begin_keyboard_move" => Rc::new(begin_keyboard_move),

        // Keyboard layout switching
        "next_keyboard_layout" => {
            Rc::new(|ctx| crate::keyboard_layout::cycle_keyboard_layout(ctx, true))
        }
        "prev_keyboard_layout" => {
            Rc::new(|ctx| crate::keyboard_layout::cycle_keyboard_layout(ctx, false))
        }

        _ => {
            eprintln!("instantwm: unknown action '{name}' in keybind config");
            return None;
        }
    };
    Some(action)
}

// ---------------------------------------------------------------------------
// Merge logic
// ---------------------------------------------------------------------------

/// Merge TOML keybind specs into a default keybind list.
///
/// For each spec, if a default with the same `(mod_mask, keysym)` exists it is
/// replaced (or removed if `unbind`). Otherwise the new binding is appended.
pub fn merge_keybinds(defaults: Vec<Key>, specs: &[KeybindSpec]) -> Vec<Key> {
    // Build a map keyed by (mod_mask, keysym) preserving insertion order via Vec index.
    // We use a Vec + HashMap<(u32,u32), usize> for ordering.
    let mut keys: Vec<Option<Key>> = defaults.into_iter().map(Some).collect();
    let mut index: HashMap<(u32, u32), usize> = HashMap::new();
    for (i, k) in keys.iter().enumerate() {
        if let Some(k) = k {
            // Last occurrence wins for defaults with duplicate combos
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
            ActionSpec::Structured(StructuredAction::Unbind(true)) => {
                // Remove existing binding
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
                    };
                    if let Some(&idx) = index.get(&combo) {
                        // Override existing
                        keys[idx] = Some(new_key);
                    } else {
                        // Append new
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
