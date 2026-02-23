#![allow(deprecated)]
//! Mouse button bindings.
//!
//! # Macro syntax
//!
//! The [`btn!`] macro keeps button binding definitions compact.
//!
//! ```text
//! // Spawn a command
//! btn!(LtSymbol, 0,     button:1 => spawn Cmd::InstantMenu)
//!
//! // Call a function with no argument
//! btn!(WinTitle, 0,     button:2 => close_win)
//!
//! // Call a function with an integer argument
//! btn!(WinTitle, 0,     button:5 => focus_stack i:1)
//!
//! // Call a function with an unsigned-integer argument
//! btn!(TagBar,   MODKEY, button:1 => tag ui:0)
//!
//! // Call a function with a layout-index argument
//! btn!(LtSymbol, 0,     button:2 => set_layout v:0)
//! ```

use super::commands::Cmd;
use super::keybindings::{CONTROL, MOD1, MODKEY, SHIFT};
use crate::animation::{down_scale_client, up_scale_client};
use crate::client::{close_win, kill_client};
use crate::focus::focus_stack;
use crate::layouts::{cycle_layout, set_layout};

use crate::floating::toggle_floating;
use crate::mouse::{
    drag_tag, draw_window, force_resize_mouse, gesture_mouse, move_mouse, resize_aspect_mouse,
    resize_mouse, window_title_mouse_handler, window_title_mouse_handler_right,
};
use crate::overlay::{create_overlay, hide_overlay, set_overlay, show_overlay};
use crate::push::{push_down, push_up};
use crate::tags::{
    follow_tag, shift_view, tag, toggle_tag, toggle_view, view_to_left, view_to_right,
};
use crate::toggles::{toggle_locked, toggle_prefix};
use crate::types::{Arg, Button, Click};
use crate::util::spawn;

// Convenience composite modifiers (mirrors keybindings.rs)
const MS: u32 = MODKEY | SHIFT;
const MC: u32 = MODKEY | CONTROL;
const MA: u32 = MODKEY | MOD1;

// ---------------------------------------------------------------------------
// btn! macro
// ---------------------------------------------------------------------------

/// Build a [`Button`] binding concisely.
///
/// Forms:
/// - `btn!(click, mask, button:N => func)`           — no argument
/// - `btn!(click, mask, button:N => func i:VAL)`     — integer arg
/// - `btn!(click, mask, button:N => func ui:VAL)`    — unsigned-integer arg
/// - `btn!(click, mask, button:N => func v:VAL)`     — usize/layout-index arg
/// - `btn!(click, mask, button:N => spawn CMD)`      — spawn a [`Cmd`]
macro_rules! btn {
    // spawn shorthand
    ($click:expr, $mask:expr, button:$btn:expr => spawn $cmd:expr) => {
        Button {
            click: $click,
            mask: $mask,
            button: $btn,
            func: Some(spawn),
            arg: Arg {
                v: Some($cmd as usize),
                ..Default::default()
            },
        }
    };
    // no argument
    ($click:expr, $mask:expr, button:$btn:expr => $func:expr) => {
        Button {
            click: $click,
            mask: $mask,
            button: $btn,
            func: Some($func),
            arg: Arg::default(),
        }
    };
    // integer arg
    ($click:expr, $mask:expr, button:$btn:expr => $func:expr, i:$val:expr) => {
        Button {
            click: $click,
            mask: $mask,
            button: $btn,
            func: Some($func),
            arg: Arg {
                i: $val,
                ..Default::default()
            },
        }
    };
    // unsigned-integer arg
    ($click:expr, $mask:expr, button:$btn:expr => $func:expr, ui:$val:expr) => {
        Button {
            click: $click,
            mask: $mask,
            button: $btn,
            func: Some($func),
            arg: Arg {
                ui: $val,
                ..Default::default()
            },
        }
    };
    // layout / usize index arg (stored in .v)
    ($click:expr, $mask:expr, button:$btn:expr => $func:expr, v:$val:expr) => {
        Button {
            click: $click,
            mask: $mask,
            button: $btn,
            func: Some($func),
            arg: Arg {
                v: Some($val),
                ..Default::default()
            },
        }
    };
}

// ---------------------------------------------------------------------------
// Button bindings
// ---------------------------------------------------------------------------

pub fn get_buttons() -> Vec<Button> {
    use Click::*;

    vec![
        // --- Layout symbol (left of bar) ---
        btn!(LtSymbol, 0,     button:1 => cycle_layout, i:-1),
        btn!(LtSymbol, 0,     button:3 => cycle_layout, i:1),
        btn!(LtSymbol, 0,     button:2 => set_layout,   v:0),
        btn!(LtSymbol, MODKEY, button:1 => create_overlay),
        // --- Window title bar ---
        btn!(WinTitle, 0,     button:1 => window_title_mouse_handler),
        btn!(WinTitle, 0,     button:2 => close_win),
        btn!(WinTitle, 0,     button:3 => window_title_mouse_handler_right),
        btn!(WinTitle, MODKEY, button:1 => set_overlay),
        btn!(WinTitle, MODKEY, button:3 => spawn Cmd::Notify),
        // Scroll to cycle focus
        btn!(WinTitle, 0,     button:4 => focus_stack, i:-1),
        btn!(WinTitle, 0,     button:5 => focus_stack, i:1),
        // Shift-scroll to reorder stack
        btn!(WinTitle, SHIFT, button:4 => push_up),
        btn!(WinTitle, SHIFT, button:5 => push_down),
        // Ctrl-scroll to scale client
        btn!(WinTitle, CONTROL, button:4 => up_scale_client),
        btn!(WinTitle, CONTROL, button:5 => down_scale_client),
        // --- Status text (right of bar) ---
        btn!(StatusText, 0,     button:1 => spawn Cmd::Panther),
        btn!(StatusText, 0,     button:2 => spawn Cmd::Term),
        btn!(StatusText, 0,     button:3 => spawn Cmd::CaretInstantSwitch),
        btn!(StatusText, 0,     button:4 => spawn Cmd::UpVol),
        btn!(StatusText, 0,     button:5 => spawn Cmd::DownVol),
        btn!(StatusText, MODKEY, button:1 => spawn Cmd::InstantSettings),
        btn!(StatusText, MODKEY, button:2 => spawn Cmd::MuteVol),
        btn!(StatusText, MODKEY, button:3 => spawn Cmd::Spoticli),
        btn!(StatusText, MODKEY, button:4 => spawn Cmd::UpBright),
        btn!(StatusText, MODKEY, button:5 => spawn Cmd::DownBright),
        btn!(StatusText, MS,     button:1 => spawn Cmd::PavuControl),
        btn!(StatusText, MC,     button:1 => spawn Cmd::Notify),
        // --- Tag bar ---
        btn!(TagBar, 0,     button:1 => drag_tag),
        btn!(TagBar, 0,     button:3 => toggle_view),
        btn!(TagBar, 0,     button:4 => view_to_left),
        btn!(TagBar, 0,     button:5 => view_to_right),
        btn!(TagBar, MODKEY, button:1 => tag),
        btn!(TagBar, MODKEY, button:3 => toggle_tag),
        btn!(TagBar, MOD1,   button:1 => follow_tag),
        btn!(TagBar, MODKEY, button:4 => shift_view, i:-1),
        btn!(TagBar, MODKEY, button:5 => shift_view, i:1),
        // --- Root window (desktop) ---
        btn!(RootWin, 0,     button:1 => spawn Cmd::Panther),
        btn!(RootWin, 0,     button:2 => spawn Cmd::InstantMenu),
        btn!(RootWin, 0,     button:3 => spawn Cmd::Smart),
        btn!(RootWin, 0,     button:4 => hide_overlay),
        btn!(RootWin, 0,     button:5 => show_overlay),
        btn!(RootWin, MODKEY, button:1 => set_overlay),
        btn!(RootWin, MODKEY, button:3 => spawn Cmd::Notify),
        // --- Client window ---
        btn!(ClientWin, MODKEY, button:1 => move_mouse),
        btn!(ClientWin, MODKEY, button:2 => toggle_floating),
        btn!(ClientWin, MODKEY, button:3 => resize_mouse),
        btn!(ClientWin, MA,     button:3 => force_resize_mouse),
        btn!(ClientWin, MS,     button:3 => resize_aspect_mouse),
        // --- Close button widget ---
        btn!(CloseButton, 0, button:1 => kill_client),
        btn!(CloseButton, 0, button:3 => toggle_locked),
        // --- Resize widget ---
        btn!(ResizeWidget, 0, button:1 => draw_window),
        // --- Shutdown button ---
        btn!(ShutDown, 0, button:1 => spawn Cmd::InstantShutdown),
        btn!(ShutDown, 0, button:2 => spawn Cmd::OsLock),
        btn!(ShutDown, 0, button:3 => spawn Cmd::Slock),
        // --- Sidebar (gesture area) ---
        btn!(SideBar, 0, button:1 => gesture_mouse),
        // --- Start menu button ---
        btn!(StartMenu, 0,     button:1 => spawn Cmd::StartMenu),
        btn!(StartMenu, 0,     button:3 => spawn Cmd::QuickMenu),
        btn!(StartMenu, SHIFT, button:1 => toggle_prefix),
    ]
}
