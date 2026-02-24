#![allow(deprecated)]
//! Mouse button bindings.

use std::rc::Rc;

use super::commands::Cmd;
use super::keybindings::{CONTROL, MOD1, MODKEY, SHIFT};
use crate::animation::{down_scale_client, up_scale_client};
use crate::client::{close_win, kill_client};
use crate::focus::focus_stack;
use crate::layouts::{cycle_layout_direction, set_layout};

use crate::floating::toggle_floating;
use crate::mouse::{
    drag_tag, draw_window, gesture_mouse, move_mouse, resize_aspect_mouse,
    resize_mouse_from_cursor, window_title_mouse_handler, window_title_mouse_handler_right,
};
use crate::overlay::{create_overlay, hide_overlay, set_overlay, show_overlay};
use crate::push::{push_down, push_up};
use crate::tags::{
    follow_tag, set_client_tag, shift_view, toggle_tag, toggle_view, view_to_left, view_to_right,
};
use crate::toggles::{toggle_locked, toggle_prefix};
use crate::types::{Button, Click, Direction, StackDirection, TagMask};
use crate::util::spawn;

const MS: u32 = MODKEY | SHIFT;
const MC: u32 = MODKEY | CONTROL;
const MA: u32 = MODKEY | MOD1;

macro_rules! btn {
    ($click:expr, $mask:expr, button:$btn:expr => $action:expr) => {
        Button {
            click: $click,
            mask: $mask,
            button: $btn,
            action: Rc::new(Box::new($action)),
        }
    };
}

pub fn get_buttons() -> Vec<Button> {
    use Click::*;

    vec![
        btn!(LtSymbol, 0,     button:1 => || cycle_layout_direction(false)),
        btn!(LtSymbol, 0,     button:3 => || cycle_layout_direction(true)),
        btn!(LtSymbol, 0,     button:2 => || set_layout(Some(0))),
        btn!(LtSymbol, MODKEY, button:1 => create_overlay),
        btn!(WinTitle, 0,     button:1 => window_title_mouse_handler),
        btn!(WinTitle, 0,     button:2 => close_win),
        btn!(WinTitle, 0,     button:3 => window_title_mouse_handler_right),
        btn!(WinTitle, MODKEY, button:1 => set_overlay),
        btn!(WinTitle, MODKEY, button:3 => || spawn(Cmd::Notify)),
        btn!(WinTitle, 0,     button:4 => || focus_stack(StackDirection::Previous)),
        btn!(WinTitle, 0,     button:5 => || focus_stack(StackDirection::Next)),
        btn!(WinTitle, SHIFT, button:4 => push_up),
        btn!(WinTitle, SHIFT, button:5 => push_down),
        btn!(WinTitle, CONTROL, button:4 => up_scale_client),
        btn!(WinTitle, CONTROL, button:5 => down_scale_client),
        btn!(StatusText, 0,     button:1 => || spawn(Cmd::Panther)),
        btn!(StatusText, 0,     button:2 => || spawn(Cmd::Term)),
        btn!(StatusText, 0,     button:3 => || spawn(Cmd::CaretInstantSwitch)),
        btn!(StatusText, 0,     button:4 => || spawn(Cmd::UpVol)),
        btn!(StatusText, 0,     button:5 => || spawn(Cmd::DownVol)),
        btn!(StatusText, MODKEY, button:1 => || spawn(Cmd::InstantSettings)),
        btn!(StatusText, MODKEY, button:2 => || spawn(Cmd::MuteVol)),
        btn!(StatusText, MODKEY, button:3 => || spawn(Cmd::Spoticli)),
        btn!(StatusText, MODKEY, button:4 => || spawn(Cmd::UpBright)),
        btn!(StatusText, MODKEY, button:5 => || spawn(Cmd::DownBright)),
        btn!(StatusText, MS,     button:1 => || spawn(Cmd::PavuControl)),
        btn!(StatusText, MC,     button:1 => || spawn(Cmd::Notify)),
        btn!(TagBar, 0,     button:1 => drag_tag),
        btn!(TagBar, 0,     button:3 => || toggle_view(TagMask::ALL_BITS)),
        btn!(TagBar, 0,     button:4 => view_to_left),
        btn!(TagBar, 0,     button:5 => view_to_right),
        btn!(TagBar, MODKEY, button:1 => || set_client_tag(TagMask::ALL_BITS)),
        btn!(TagBar, MODKEY, button:3 => || toggle_tag(TagMask::ALL_BITS)),
        btn!(TagBar, MOD1,   button:1 => || follow_tag(TagMask::ALL_BITS)),
        btn!(TagBar, MODKEY, button:4 => || shift_view(Direction::Left)),
        btn!(TagBar, MODKEY, button:5 => || shift_view(Direction::Right)),
        btn!(RootWin, 0,     button:1 => || spawn(Cmd::Panther)),
        btn!(RootWin, 0,     button:2 => || spawn(Cmd::InstantMenu)),
        btn!(RootWin, 0,     button:3 => || spawn(Cmd::Smart)),
        btn!(RootWin, 0,     button:4 => hide_overlay),
        btn!(RootWin, 0,     button:5 => show_overlay),
        btn!(RootWin, MODKEY, button:1 => set_overlay),
        btn!(RootWin, MODKEY, button:3 => || spawn(Cmd::Notify)),
        btn!(ClientWin, MODKEY, button:1 => move_mouse),
        btn!(ClientWin, MODKEY, button:2 => toggle_floating),
        btn!(ClientWin, MODKEY, button:3 => resize_mouse_from_cursor),
        btn!(ClientWin, MA,     button:3 => resize_mouse_from_cursor),
        btn!(ClientWin, MS,     button:3 => resize_aspect_mouse),
        btn!(CloseButton, 0, button:1 => kill_client),
        btn!(CloseButton, 0, button:3 => toggle_locked),
        btn!(ResizeWidget, 0, button:1 => draw_window),
        btn!(ShutDown, 0, button:1 => || spawn(Cmd::InstantShutdown)),
        btn!(ShutDown, 0, button:2 => || spawn(Cmd::OsLock)),
        btn!(ShutDown, 0, button:3 => || spawn(Cmd::Slock)),
        btn!(SideBar, 0, button:1 => gesture_mouse),
        btn!(StartMenu, 0,     button:1 => || spawn(Cmd::StartMenu)),
        btn!(StartMenu, 0,     button:3 => || spawn(Cmd::QuickMenu)),
        btn!(StartMenu, SHIFT, button:1 => toggle_prefix),
    ]
}
