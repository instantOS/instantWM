//! Keyboard bindings: normal keys (`get_keys`) and prefix-mode keys (`get_dkeys`).
//!
//! # Macro syntax
//!
//! The [`key!`] macro keeps binding definitions compact.  Each form maps
//! directly to a [`Key`] struct:
//!
//! ```text
//! // Spawn a command
//! key!(MOD, XK_t => spawn CMD_TERM)
//!
//! // Call a function with no argument
//! key!(MOD, XK_Tab => last_view)
//!
//! // Call a function with an integer argument
//! key!(MOD, XK_j => focus_stack i:1)
//!
//! // Call a function with an unsigned-integer argument
//! key!(MOD, XK_e => toggle_overview ui:!0)
//!
//! // Call a function with a float argument
//! key!(MOD, XK_h => set_mfact f:-0.05)
//!
//! // Call a function with a layout-index argument
//! key!(MOD, XK_t => set_layout v:0)
//! ```

use super::commands::Cmd;
use crate::animation::{anim_left, anim_right, down_scale_client, up_scale_client};
use crate::bar::toggle_bar;
use crate::client::{kill_client, shut_kill, toggle_fake_fullscreen, zoom};
use crate::commands::{command_prefix, set_special_next};
use crate::floating::{center_window, distribute_clients, temp_fullscreen, toggle_floating};
use crate::focus::{direction_focus, focus_last_client, focus_stack, warp_to_focus};
use crate::keyboard::{
    down_key, down_press, focus_nmon, key_resize, space_toggle, up_key, up_press,
};
use crate::layouts::{command_layout, cycle_layout, inc_nmaster, set_layout, set_mfact};
use crate::monitor::{focus_mon, follow_mon};
use crate::mouse::{drag_tag, draw_window, move_mouse, moveresize, resize_mouse};
use crate::overlay::{create_overlay, hide_overlay, set_overlay, show_overlay};
use crate::push::{push_down, push_up};
use crate::scratchpad::{scratchpad_make, scratchpad_toggle};
use crate::tags::{
    desktop_set, follow_tag, follow_view, last_view, move_left, move_right, name_tag, quit,
    reset_name_tag, shift_view, swap_tags, tag, tag_mon, tag_to_left, tag_to_right,
    toggle_fullscreen_overview, toggle_overview, toggle_tag, toggle_view, view, view_to_left,
    view_to_right, win_view,
};
use crate::toggles::{
    alt_tab_free, hide_window, redraw_win, set_border_width, toggle_alt_tag, toggle_animated,
    toggle_double_draw, toggle_focus_follows_float_mouse, toggle_focus_follows_mouse,
    toggle_locked, toggle_prefix, toggle_show_tags, toggle_sticky, unhide_all,
};
use crate::types::{Arg, Key};
use crate::util::spawn;

use super::keysyms::*;

// ---------------------------------------------------------------------------
// Modifier aliases
// ---------------------------------------------------------------------------

/// Super / Windows key (Mod4).
pub const MODKEY: u32 = 1 << 6;
/// Control modifier.
pub const CONTROL: u32 = 1 << 2;
/// Shift modifier.
pub const SHIFT: u32 = 1 << 0;
/// Alt modifier (Mod1).
pub const MOD1: u32 = 1 << 3;

// ---------------------------------------------------------------------------
// key! macro
// ---------------------------------------------------------------------------

/// Build a [`Key`] binding concisely.
///
/// Forms:
/// - `key!(mods, sym => func)`              — no argument
/// - `key!(mods, sym => func i:VAL)`        — integer arg
/// - `key!(mods, sym => func ui:VAL)`       — unsigned-integer arg
/// - `key!(mods, sym => func f:VAL)`        — float arg
/// - `key!(mods, sym => func v:VAL)`        — usize/layout-index arg
/// - `key!(mods, sym => spawn CMD)`         — spawn with a [`Cmd`] variant
macro_rules! key {
    // spawn shorthand
    ($mods:expr, $sym:expr => spawn $cmd:expr) => {
        Key {
            mod_mask: $mods,
            keysym: $sym,
            func: Some(spawn),
            arg: Arg {
                v: Some($cmd as usize),
                ..Default::default()
            },
        }
    };
    // no argument
    ($mods:expr, $sym:expr => $func:expr) => {
        Key {
            mod_mask: $mods,
            keysym: $sym,
            func: Some($func),
            arg: Arg::default(),
        }
    };
    // integer arg
    ($mods:expr, $sym:expr => $func:expr, i:$val:expr) => {
        Key {
            mod_mask: $mods,
            keysym: $sym,
            func: Some($func),
            arg: Arg {
                i: $val,
                ..Default::default()
            },
        }
    };
    // unsigned-integer arg
    ($mods:expr, $sym:expr => $func:expr, ui:$val:expr) => {
        Key {
            mod_mask: $mods,
            keysym: $sym,
            func: Some($func),
            arg: Arg {
                ui: $val,
                ..Default::default()
            },
        }
    };
    // float arg
    ($mods:expr, $sym:expr => $func:expr, f:$val:expr) => {
        Key {
            mod_mask: $mods,
            keysym: $sym,
            func: Some($func),
            arg: Arg {
                f: $val,
                ..Default::default()
            },
        }
    };
    // layout / usize index arg (stored in .v)
    ($mods:expr, $sym:expr => $func:expr, v:$val:expr) => {
        Key {
            mod_mask: $mods,
            keysym: $sym,
            func: Some($func),
            arg: Arg {
                v: Some($val),
                ..Default::default()
            },
        }
    };
}

// ---------------------------------------------------------------------------
// Per-tag binding generator
// ---------------------------------------------------------------------------

/// Emit the six standard bindings for tag number `tag_idx` (0-based),
/// bound to `keysym`.
fn tag_keys(keysym: u32, tag_idx: usize) -> [Key; 6] {
    let mask = 1u32 << tag_idx;
    [
        key!(MODKEY,                    keysym => view,        ui:mask),
        key!(MODKEY | CONTROL,          keysym => toggle_view, ui:mask),
        key!(MODKEY | SHIFT,            keysym => tag,         ui:mask),
        key!(MODKEY | MOD1,             keysym => follow_tag,  ui:mask),
        key!(MODKEY | CONTROL | SHIFT,  keysym => toggle_tag,  ui:mask),
        key!(MODKEY | MOD1   | SHIFT,   keysym => swap_tags,   ui:mask),
    ]
}

// Convenience composite modifiers
const MS: u32 = MODKEY | SHIFT;
const MC: u32 = MODKEY | CONTROL;
const MA: u32 = MODKEY | MOD1;
const MCA: u32 = MODKEY | CONTROL | MOD1;
const MSC: u32 = MODKEY | SHIFT | CONTROL;
const MSA: u32 = MODKEY | SHIFT | MOD1;
const MSCA: u32 = MODKEY | SHIFT | CONTROL | MOD1;

// ---------------------------------------------------------------------------
// Normal key bindings
// ---------------------------------------------------------------------------

/// All keybindings active outside of prefix mode.
pub fn get_keys() -> Vec<Key> {
    let mut keys: Vec<Key> = vec![
        // --- Resize focused tiled window with keyboard ---
        key!(MA, XK_j => key_resize, i:0),
        key!(MA, XK_k => key_resize, i:1),
        key!(MA, XK_l => key_resize, i:2),
        key!(MA, XK_h => key_resize, i:3),
        // --- Layout / master factor ---
        key!(MODKEY, XK_i => inc_nmaster, i:1),
        key!(MODKEY, XK_d => inc_nmaster, i:-1),
        key!(MODKEY, XK_h => set_mfact,   f:-0.05),
        key!(MODKEY, XK_l => set_mfact,   f:0.05),
        // --- Layout selection ---
        key!(MODKEY,    XK_t => set_layout, v:0), // tile
        key!(MODKEY,    XK_c => set_layout, v:1), // grid
        key!(MODKEY,    XK_f => set_layout, v:2), // float
        key!(MODKEY,    XK_m => set_layout, v:3), // monocle
        key!(MODKEY,    XK_p => set_layout),
        key!(MC,        XK_comma  => cycle_layout, i:-1),
        key!(MC,        XK_period => cycle_layout, i:1),
        // --- Focus movement ---
        key!(MODKEY, XK_j    => focus_stack, i:1),
        key!(MODKEY, XK_k    => focus_stack, i:-1),
        key!(MODKEY, XK_Down => down_key,    i:1),
        key!(MODKEY, XK_Up   => up_key,      i:-1),
        key!(MS,     XK_Down => down_press),
        key!(MS,     XK_Up   => up_press),
        // --- Stack order ---
        key!(MC, XK_j => push_down),
        key!(MC, XK_k => push_up),
        // --- Directional focus ---
        key!(MC, XK_Left  => direction_focus, ui:3),
        key!(MC, XK_Right => direction_focus, ui:1),
        key!(MC, XK_Up    => direction_focus, ui:0),
        key!(MC, XK_Down  => direction_focus, ui:2),
        // --- Tag navigation ---
        key!(MODKEY,  XK_Tab     => last_view),
        key!(MS,      XK_Tab     => focus_last_client),
        key!(MA,      XK_Tab     => follow_view),
        key!(MODKEY,  XK_Left    => anim_left),
        key!(MODKEY,  XK_Right   => anim_right),
        key!(MA,      XK_Left    => move_left),
        key!(MA,      XK_Right   => move_right),
        key!(MS,      XK_Left    => tag_to_left),
        key!(MS,      XK_Right   => tag_to_right),
        key!(MSC,     XK_Right   => shift_view, i:1),
        key!(MSC,     XK_Left    => shift_view, i:-1),
        key!(MODKEY,  XK_0       => view,  ui:!0),
        key!(MS,      XK_0       => tag,   ui:!0),
        key!(MODKEY,  XK_o       => win_view),
        // --- Monitor focus ---
        key!(MODKEY, XK_comma  => focus_mon,  i:-1),
        key!(MODKEY, XK_period => focus_mon,  i:1),
        key!(MS,     XK_comma  => tag_mon,    i:-1),
        key!(MS,     XK_period => tag_mon,    i:1),
        key!(MA,     XK_comma  => follow_mon, i:-1),
        key!(MA,     XK_period => follow_mon, i:1),
        key!(MSCA,   XK_period => desktop_set),
        // --- Float / window management ---
        key!(MS,   XK_Return => zoom),
        key!(MC,   XK_d      => distribute_clients),
        key!(MS,   XK_d      => draw_window),
        key!(MA,   XK_w      => center_window),
        key!(MS,   XK_w      => warp_to_focus),
        key!(MS,   XK_j      => moveresize, i:0),
        key!(MS,   XK_k      => moveresize, i:1),
        key!(MS,   XK_l      => moveresize, i:2),
        key!(MS,   XK_h      => moveresize, i:3),
        key!(MS,   XK_m      => move_mouse),
        key!(MA,   XK_m      => resize_mouse),
        // --- Overview / skippy ---
        key!(MODKEY, XK_e  => toggle_overview,           ui:!0),
        key!(MS,     XK_e  => toggle_fullscreen_overview, ui:!0),
        key!(MC,     XK_e  => spawn Cmd::InstantSkippy),
        // --- Overlays ---
        key!(MODKEY, XK_w  => set_overlay),
        key!(MC,     XK_w  => create_overlay),
        // --- Scratchpad ---
        key!(MODKEY, XK_s  => scratchpad_toggle, v:Cmd::Default as usize),
        key!(MS,     XK_s  => scratchpad_make,   v:Cmd::Default as usize),
        // --- Toggles ---
        key!(MODKEY, XK_b  => toggle_bar),
        key!(MS,     XK_f  => toggle_fake_fullscreen),
        key!(MC,     XK_f  => temp_fullscreen),
        key!(MC,     XK_s  => toggle_sticky),
        key!(MA,     XK_s  => toggle_alt_tag,  ui:2),
        key!(MSA,    XK_s  => toggle_animated, ui:2),
        key!(MSC,    XK_s  => toggle_show_tags, ui:2),
        key!(MSA,    XK_d  => toggle_double_draw),
        key!(MS,     XK_space => space_toggle),
        key!(MSCA,   XK_Tab   => alt_tab_free),
        key!(MC,     XK_r     => redraw_win),
        // --- Hiding ---
        key!(MC,  XK_h => hide_window),
        key!(MCA, XK_h => unhide_all),
        // --- Close / quit ---
        key!(MODKEY, XK_q   => shut_kill),
        key!(MOD1,   XK_F4  => kill_client),
        key!(MSC,    XK_q   => quit),
        // --- Misc keys ---
        key!(MODKEY,  XK_F1 => spawn Cmd::Help),
        key!(MODKEY,  XK_F2 => toggle_prefix),
        // --- Launchers ---
        key!(MODKEY,      XK_Return         => spawn Cmd::Term),
        key!(MODKEY,      XK_space          => spawn Cmd::Smart),
        key!(MC,          XK_space          => spawn Cmd::InstantMenu),
        key!(MS,          XK_v              => spawn Cmd::ClipMenu),
        key!(MODKEY,      XK_minus          => spawn Cmd::InstantMenuSt),
        key!(MODKEY,      XK_v              => spawn Cmd::QuickMenu),
        key!(MODKEY,      XK_a              => spawn Cmd::InstantAssist),
        key!(MS,          XK_a              => spawn Cmd::InstantRepeat),
        key!(MC,          XK_i              => spawn Cmd::InstantPacman),
        key!(MS,          XK_i              => spawn Cmd::InstantShare),
        key!(MODKEY,      XK_n              => spawn Cmd::Nautilus),
        key!(MODKEY,      XK_r              => spawn Cmd::Yazi),
        key!(MODKEY,      XK_y              => spawn Cmd::Panther),
        key!(MODKEY,      XK_g              => spawn Cmd::Notify),
        key!(MODKEY,      XK_x              => spawn Cmd::InstantSwitch),
        key!(MOD1,        XK_Tab            => spawn Cmd::ISwitch),
        key!(MODKEY,      XK_dead_circumflex => spawn Cmd::CaretInstantSwitch),
        key!(MA,          XK_f              => spawn Cmd::Search),
        key!(MA,          XK_space          => spawn Cmd::KeyLayoutSwitch),
        key!(MCA,         XK_l              => spawn Cmd::LangSwitch),
        key!(MC,          XK_l              => spawn Cmd::Slock),
        key!(MSC,         XK_l              => spawn Cmd::OneKeyLock),
        key!(MC,          XK_q              => spawn Cmd::InstantShutdown),
        key!(MS,          XK_Escape         => spawn Cmd::SystemMonitor),
        key!(MC,          XK_c              => spawn Cmd::ControlCenter),
        key!(MS,          XK_p              => spawn Cmd::Display),
        // --- Screenshot ---
        key!(MODKEY, XK_Print  => spawn Cmd::Scrot),
        key!(MS,     XK_Print  => spawn Cmd::FScrot),
        key!(MC,     XK_Print  => spawn Cmd::ClipScrot),
        key!(MA,     XK_Print  => spawn Cmd::FClipScrot),
        // --- Media / hardware keys (no modifier) ---
        key!(0, XF86XK_MonBrightnessUp    => spawn Cmd::UpBright),
        key!(0, XF86XK_MonBrightnessDown  => spawn Cmd::DownBright),
        key!(0, XF86XK_AudioLowerVolume   => spawn Cmd::DownVol),
        key!(0, XF86XK_AudioMute          => spawn Cmd::MuteVol),
        key!(0, XF86XK_AudioRaiseVolume   => spawn Cmd::UpVol),
        key!(0, XF86XK_AudioPlay          => spawn Cmd::PlayerPause),
        key!(0, XF86XK_AudioPause         => spawn Cmd::PlayerPause),
        key!(0, XF86XK_AudioNext          => spawn Cmd::PlayerNext),
        key!(0, XF86XK_AudioPrev          => spawn Cmd::PlayerPrevious),
    ];

    // Tag keys 1–9
    for tag_idx in 0..9 {
        keys.extend_from_slice(&tag_keys(XK_1 + tag_idx as u32, tag_idx));
    }

    keys
}

// ---------------------------------------------------------------------------
// Prefix-mode (dkey) bindings
// ---------------------------------------------------------------------------

/// Keybindings active while in prefix mode (activated by `toggle_prefix`).
///
/// In prefix mode the modifier is irrelevant — `mod_mask` is always `0`.
pub fn get_dkeys() -> Vec<Key> {
    vec![
        // --- Launchers ---
        key!(0, XK_Return => spawn Cmd::Term),
        key!(0, XK_r      => spawn Cmd::Yazi),
        key!(0, XK_e      => spawn Cmd::Editor),
        key!(0, XK_n      => spawn Cmd::Nautilus),
        key!(0, XK_space  => spawn Cmd::Panther),
        key!(0, XK_f      => spawn Cmd::Firefox),
        key!(0, XK_a      => spawn Cmd::InstantAssist),
        key!(0, XK_F1     => spawn Cmd::Help),
        key!(0, XK_m      => spawn Cmd::Spoticli),
        key!(0, XK_c      => spawn Cmd::Code),
        key!(0, XK_y      => spawn Cmd::Smart),
        key!(0, XK_v      => spawn Cmd::QuickMenu),
        key!(0, XK_Tab    => spawn Cmd::CaretInstantSwitch),
        key!(0, XK_plus   => spawn Cmd::UpVol),
        key!(0, XK_minus  => spawn Cmd::DownVol),
        // --- Tag navigation ---
        key!(0, XK_h     => view_to_left),
        key!(0, XK_l     => view_to_right),
        key!(0, XK_Left  => view_to_left),
        key!(0, XK_Right => view_to_right),
        key!(0, XK_k     => shift_view, i:1),
        key!(0, XK_j     => shift_view, i:-1),
        key!(0, XK_Up    => shift_view, i:1),
        key!(0, XK_Down  => shift_view, i:-1),
        // --- Direct tag jump (1-9) ---
        key!(0, XK_1 => view, ui:1 << 0),
        key!(0, XK_2 => view, ui:1 << 1),
        key!(0, XK_3 => view, ui:1 << 2),
        key!(0, XK_4 => view, ui:1 << 3),
        key!(0, XK_5 => view, ui:1 << 4),
        key!(0, XK_6 => view, ui:1 << 5),
        key!(0, XK_7 => view, ui:1 << 6),
        key!(0, XK_8 => view, ui:1 << 7),
        key!(0, XK_9 => view, ui:1 << 8),
    ]
}
