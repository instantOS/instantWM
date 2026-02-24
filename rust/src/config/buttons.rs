#![allow(deprecated)]
//! Mouse button bindings.

use std::rc::Rc;

use super::commands::Cmd;
use super::keybindings::{CONTROL, MOD1, MODKEY, SHIFT};
use crate::animation::{down_scale_client, up_scale_client};
use crate::client::{close_win, kill_client};
use crate::focus::focus_stack;
use crate::layouts::{cycle_layout_direction, set_layout, LayoutKind};

use crate::floating::toggle_floating;
use crate::mouse::{
    drag_tag, draw_window, gesture_mouse, move_mouse, resize_aspect_mouse,
    resize_mouse_from_cursor, window_title_mouse_handler, window_title_mouse_handler_right,
};
use crate::overlay::{create_overlay, hide_overlay, set_overlay, show_overlay};
use crate::push::{push_down, push_up};
use crate::tags::{follow_tag, set_client_tag, shift_view, toggle_tag, toggle_view};
use crate::toggles::{toggle_locked, toggle_prefix};
use crate::types::{Button, Click, Direction, MouseButton, StackDirection, TagMask};
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
        btn!(LtSymbol, 0,     button:MouseButton::Left => || cycle_layout_direction(false)),
        btn!(LtSymbol, 0,     button:MouseButton::Right => || cycle_layout_direction(true)),
        btn!(LtSymbol, 0,     button:MouseButton::Middle => || set_layout(LayoutKind::Tile)),
        btn!(LtSymbol, MODKEY, button:MouseButton::Left => create_overlay),
        btn!(WinTitle, 0,     button:MouseButton::Left => window_title_mouse_handler),
        btn!(WinTitle, 0,     button:MouseButton::Middle => close_win),
        btn!(WinTitle, 0,     button:MouseButton::Right => window_title_mouse_handler_right),
        btn!(WinTitle, MODKEY, button:MouseButton::Left => set_overlay),
        btn!(WinTitle, MODKEY, button:MouseButton::Right => || spawn(Cmd::Notify)),
        btn!(WinTitle, 0,     button:MouseButton::ScrollUp => || focus_stack(StackDirection::Previous)),
        btn!(WinTitle, 0,     button:MouseButton::ScrollDown => || focus_stack(StackDirection::Next)),
        btn!(WinTitle, SHIFT, button:MouseButton::ScrollUp => push_up),
        btn!(WinTitle, SHIFT, button:MouseButton::ScrollDown => push_down),
        btn!(WinTitle, CONTROL, button:MouseButton::ScrollUp => up_scale_client),
        btn!(WinTitle, CONTROL, button:MouseButton::ScrollDown => down_scale_client),
        btn!(StatusText, 0,     button:MouseButton::Left => || spawn(Cmd::Panther)),
        btn!(StatusText, 0,     button:MouseButton::Middle => || spawn(Cmd::Term)),
        btn!(StatusText, 0,     button:MouseButton::Right => || spawn(Cmd::CaretInstantSwitch)),
        btn!(StatusText, 0,     button:MouseButton::ScrollUp => || spawn(Cmd::UpVol)),
        btn!(StatusText, 0,     button:MouseButton::ScrollDown => || spawn(Cmd::DownVol)),
        btn!(StatusText, MODKEY, button:MouseButton::Left => || spawn(Cmd::InstantSettings)),
        btn!(StatusText, MODKEY, button:MouseButton::Middle => || spawn(Cmd::MuteVol)),
        btn!(StatusText, MODKEY, button:MouseButton::Right => || spawn(Cmd::Spoticli)),
        btn!(StatusText, MODKEY, button:MouseButton::ScrollUp => || spawn(Cmd::UpBright)),
        btn!(StatusText, MODKEY, button:MouseButton::ScrollDown => || spawn(Cmd::DownBright)),
        btn!(StatusText, MS,     button:MouseButton::Left => || spawn(Cmd::PavuControl)),
        btn!(StatusText, MC,     button:MouseButton::Left => || spawn(Cmd::Notify)),
        btn!(TagBar, 0,     button:MouseButton::Left => drag_tag),
        btn!(TagBar, 0,     button:MouseButton::Right => || toggle_view(TagMask::ALL_BITS)),
        btn!(TagBar, 0,     button:MouseButton::ScrollUp => || crate::tags::view::scroll_view(Direction::Left)),
        btn!(TagBar, 0,     button:MouseButton::ScrollDown => || crate::tags::view::scroll_view(Direction::Right)),
        btn!(TagBar, MODKEY, button:MouseButton::Left => || set_client_tag(TagMask::ALL_BITS)),
        btn!(TagBar, MODKEY, button:MouseButton::Right => || toggle_tag(TagMask::ALL_BITS)),
        btn!(TagBar, MOD1,   button:MouseButton::Left => || follow_tag(TagMask::ALL_BITS)),
        btn!(TagBar, MODKEY, button:MouseButton::ScrollUp => || shift_view(Direction::Left)),
        btn!(TagBar, MODKEY, button:MouseButton::ScrollDown => || shift_view(Direction::Right)),
        btn!(RootWin, 0,     button:MouseButton::Left => || spawn(Cmd::Panther)),
        btn!(RootWin, 0,     button:MouseButton::Middle => || spawn(Cmd::InstantMenu)),
        btn!(RootWin, 0,     button:MouseButton::Right => || spawn(Cmd::Smart)),
        btn!(RootWin, 0,     button:MouseButton::ScrollUp => hide_overlay),
        btn!(RootWin, 0,     button:MouseButton::ScrollDown => show_overlay),
        btn!(RootWin, MODKEY, button:MouseButton::Left => set_overlay),
        btn!(RootWin, MODKEY, button:MouseButton::Right => || spawn(Cmd::Notify)),
        btn!(ClientWin, MODKEY, button:MouseButton::Left => move_mouse),
        btn!(ClientWin, MODKEY, button:MouseButton::Middle => toggle_floating),
        btn!(ClientWin, MODKEY, button:MouseButton::Right => resize_mouse_from_cursor),
        btn!(ClientWin, MA,     button:MouseButton::Right => resize_mouse_from_cursor),
        btn!(ClientWin, MS,     button:MouseButton::Right => resize_aspect_mouse),
        btn!(CloseButton, 0, button:MouseButton::Left => kill_client),
        btn!(CloseButton, 0, button:MouseButton::Right => toggle_locked),
        btn!(ResizeWidget, 0, button:MouseButton::Left => draw_window),
        btn!(ShutDown, 0, button:MouseButton::Left => || spawn(Cmd::InstantShutdown)),
        btn!(ShutDown, 0, button:MouseButton::Middle => || spawn(Cmd::OsLock)),
        btn!(ShutDown, 0, button:MouseButton::Right => || spawn(Cmd::Slock)),
        btn!(SideBar, 0, button:MouseButton::Left => gesture_mouse),
        btn!(StartMenu, 0,     button:MouseButton::Left => || spawn(Cmd::StartMenu)),
        btn!(StartMenu, 0,     button:MouseButton::Right => || spawn(Cmd::QuickMenu)),
        btn!(StartMenu, SHIFT, button:MouseButton::Left => toggle_prefix),
    ]
}
