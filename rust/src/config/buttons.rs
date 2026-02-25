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
        btn!(LtSymbol, 0,     button:MouseButton::Left => |ctx| cycle_layout_direction(ctx, false)),
        btn!(LtSymbol, 0,     button:MouseButton::Right => |ctx| cycle_layout_direction(ctx, true)),
        btn!(LtSymbol, 0,     button:MouseButton::Middle => |ctx| set_layout(ctx, LayoutKind::Tile)),
        btn!(LtSymbol, MODKEY, button:MouseButton::Left => |ctx| {
            if let Some(win) = crate::client::selected_window(ctx) {
                create_overlay(ctx, win)
            }
        }),
        btn!(WinTitle, 0,     button:MouseButton::Left => |ctx| {
            if let Some(win) = crate::client::selected_window(ctx) {
                window_title_mouse_handler(ctx, win)
            }
        }),
        btn!(WinTitle, 0,     button:MouseButton::Middle => |ctx| {
            if let Some(win) = crate::client::selected_window(ctx) {
                close_win(ctx, win)
            }
        }),
        btn!(WinTitle, 0,     button:MouseButton::Right => |ctx| {
            if let Some(win) = crate::client::selected_window(ctx) {
                window_title_mouse_handler_right(ctx, win)
            }
        }),
        btn!(WinTitle, MODKEY, button:MouseButton::Left => set_overlay),
        btn!(WinTitle, MODKEY, button:MouseButton::Right => |ctx| spawn(ctx, Cmd::Notify)),
        btn!(WinTitle, 0,     button:MouseButton::ScrollUp => |ctx| focus_stack(ctx, StackDirection::Previous)),
        btn!(WinTitle, 0,     button:MouseButton::ScrollDown => |ctx| focus_stack(ctx, StackDirection::Next)),
        btn!(WinTitle, SHIFT, button:MouseButton::ScrollUp => |ctx| {
            if let Some(win) = crate::client::selected_window(ctx) {
                push_up(ctx, win)
            }
        }),
        btn!(WinTitle, SHIFT, button:MouseButton::ScrollDown => |ctx| {
            if let Some(win) = crate::client::selected_window(ctx) {
                push_down(ctx, win)
            }
        }),
        btn!(WinTitle, CONTROL, button:MouseButton::ScrollUp => |ctx| {
            if let Some(win) = crate::client::selected_window(ctx) {
                up_scale_client(ctx, win)
            }
        }),
        btn!(WinTitle, CONTROL, button:MouseButton::ScrollDown => |ctx| {
            if let Some(win) = crate::client::selected_window(ctx) {
                down_scale_client(ctx, win)
            }
        }),
        btn!(StatusText, 0,     button:MouseButton::Left => |ctx| spawn(ctx, Cmd::Panther)),
        btn!(StatusText, 0,     button:MouseButton::Middle => |ctx| spawn(ctx, Cmd::Term)),
        btn!(StatusText, 0,     button:MouseButton::Right => |ctx| spawn(ctx, Cmd::CaretInstantSwitch)),
        btn!(StatusText, 0,     button:MouseButton::ScrollUp => |ctx| spawn(ctx, Cmd::UpVol)),
        btn!(StatusText, 0,     button:MouseButton::ScrollDown => |ctx| spawn(ctx, Cmd::DownVol)),
        btn!(StatusText, MODKEY, button:MouseButton::Left => |ctx| spawn(ctx, Cmd::InstantSettings)),
        btn!(StatusText, MODKEY, button:MouseButton::Middle => |ctx| spawn(ctx, Cmd::MuteVol)),
        btn!(StatusText, MODKEY, button:MouseButton::Right => |ctx| spawn(ctx, Cmd::Spoticli)),
        btn!(StatusText, MODKEY, button:MouseButton::ScrollUp => |ctx| spawn(ctx, Cmd::UpBright)),
        btn!(StatusText, MODKEY, button:MouseButton::ScrollDown => |ctx| spawn(ctx, Cmd::DownBright)),
        btn!(StatusText, MS,     button:MouseButton::Left => |ctx| spawn(ctx, Cmd::PavuControl)),
        btn!(StatusText, MC,     button:MouseButton::Left => |ctx| spawn(ctx, Cmd::Notify)),
        btn!(TagBar, 0,     button:MouseButton::Left => drag_tag),
        btn!(TagBar, 0,     button:MouseButton::Right => |ctx| toggle_view(ctx, TagMask::ALL_BITS)),
        btn!(TagBar, 0,     button:MouseButton::ScrollUp => |ctx| crate::tags::view::scroll_view(ctx, Direction::Left)),
        btn!(TagBar, 0,     button:MouseButton::ScrollDown => |ctx| crate::tags::view::scroll_view(ctx, Direction::Right)),
        btn!(TagBar, MODKEY, button:MouseButton::Left => |ctx| {
            if let Some(win) = crate::client::selected_window(ctx) {
                set_client_tag(ctx, win, TagMask::ALL_BITS)
            }
        }),
        btn!(TagBar, MODKEY, button:MouseButton::Right => |ctx| {
            if let Some(win) = crate::client::selected_window(ctx) {
                toggle_tag(ctx, win, TagMask::ALL_BITS)
            }
        }),
        btn!(TagBar, MOD1,   button:MouseButton::Left => |ctx| {
            if let Some(win) = crate::client::selected_window(ctx) {
                follow_tag(ctx, win, TagMask::ALL_BITS)
            }
        }),
        btn!(TagBar, MODKEY, button:MouseButton::ScrollUp => |ctx| shift_view(ctx, Direction::Left)),
        btn!(TagBar, MODKEY, button:MouseButton::ScrollDown => |ctx| shift_view(ctx, Direction::Right)),
        btn!(RootWin, 0,     button:MouseButton::Left => |ctx| spawn(ctx, Cmd::Panther)),
        btn!(RootWin, 0,     button:MouseButton::Middle => |ctx| spawn(ctx, Cmd::InstantMenu)),
        btn!(RootWin, 0,     button:MouseButton::Right => |ctx| spawn(ctx, Cmd::Smart)),
        btn!(RootWin, 0,     button:MouseButton::ScrollUp => hide_overlay),
        btn!(RootWin, 0,     button:MouseButton::ScrollDown => show_overlay),
        btn!(RootWin, MODKEY, button:MouseButton::Left => set_overlay),
        btn!(RootWin, MODKEY, button:MouseButton::Right => |ctx| spawn(ctx, Cmd::Notify)),
        btn!(ClientWin, MODKEY, button:MouseButton::Left => move_mouse),
        btn!(ClientWin, MODKEY, button:MouseButton::Middle => toggle_floating),
        btn!(ClientWin, MODKEY, button:MouseButton::Right => resize_mouse_from_cursor),
        btn!(ClientWin, MA,     button:MouseButton::Right => resize_mouse_from_cursor),
        btn!(ClientWin, MS,     button:MouseButton::Right => |ctx| {
            if let Some(win) = crate::client::selected_window(ctx) {
                resize_aspect_mouse(ctx, win)
            }
        }),
        btn!(CloseButton, 0, button:MouseButton::Left => |ctx| {
            if let Some(win) = crate::client::selected_window(ctx) {
                kill_client(ctx, win)
            }
        }),
        btn!(CloseButton, 0, button:MouseButton::Right => |ctx| {
            if let Some(win) = crate::client::selected_window(ctx) {
                toggle_locked(ctx, win)
            }
        }),
        btn!(ResizeWidget, 0, button:MouseButton::Left => draw_window),
        btn!(ShutDown, 0, button:MouseButton::Left => |ctx| spawn(ctx, Cmd::InstantShutdown)),
        btn!(ShutDown, 0, button:MouseButton::Middle => |ctx| spawn(ctx, Cmd::OsLock)),
        btn!(ShutDown, 0, button:MouseButton::Right => |ctx| spawn(ctx, Cmd::Slock)),
        btn!(SideBar, 0, button:MouseButton::Left => gesture_mouse),
        btn!(StartMenu, 0,     button:MouseButton::Left => |ctx| spawn(ctx, Cmd::StartMenu)),
        btn!(StartMenu, 0,     button:MouseButton::Right => |ctx| spawn(ctx, Cmd::QuickMenu)),
        btn!(StartMenu, SHIFT, button:MouseButton::Left => toggle_prefix),
    ]
}
