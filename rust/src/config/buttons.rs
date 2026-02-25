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
use crate::tags::view::toggle_view_tag;
use crate::tags::{follow_tag, set_client_tag, shift_view, toggle_tag};
use crate::toggles::{toggle_locked, toggle_prefix};
use crate::types::{BarPosition, Button, Direction, MouseButton, StackDirection, TagMask};
use crate::util::spawn;

const MS: u32 = MODKEY | SHIFT;
const MC: u32 = MODKEY | CONTROL;
const MA: u32 = MODKEY | MOD1;

macro_rules! btn {
    ($target:expr, $mask:expr, button:$btn:expr => $action:expr) => {
        Button {
            target: $target,
            mask: $mask,
            button: $btn,
            action: Rc::new(Box::new($action)),
        }
    };
}

pub fn get_buttons() -> Vec<Button> {
    use BarPosition::*;

    vec![
        btn!(LtSymbol, 0,     button:MouseButton::Left   => |ctx, _| cycle_layout_direction(ctx, false)),
        btn!(LtSymbol, 0,     button:MouseButton::Right  => |ctx, _| cycle_layout_direction(ctx, true)),
        btn!(LtSymbol, 0,     button:MouseButton::Middle => |ctx, _| set_layout(ctx, LayoutKind::Tile)),
        btn!(LtSymbol, MODKEY, button:MouseButton::Left  => |ctx, _| {
            if let Some(win) = crate::client::selected_window(ctx) {
                create_overlay(ctx, win)
            }
        }),
        btn!(WinTitle(0), 0,     button:MouseButton::Left => |ctx, _| {
            if let Some(win) = crate::client::selected_window(ctx) {
                window_title_mouse_handler(ctx, win)
            }
        }),
        btn!(WinTitle(0), 0,     button:MouseButton::Middle => |ctx, _| {
            if let Some(win) = crate::client::selected_window(ctx) {
                close_win(ctx, win)
            }
        }),
        btn!(WinTitle(0), 0,     button:MouseButton::Right => |ctx, _| {
            if let Some(win) = crate::client::selected_window(ctx) {
                window_title_mouse_handler_right(ctx, win)
            }
        }),
        btn!(WinTitle(0), MODKEY, button:MouseButton::Left  => |ctx, _| set_overlay(ctx)),
        btn!(WinTitle(0), MODKEY, button:MouseButton::Right => |ctx, _| spawn(ctx, Cmd::Notify)),
        btn!(WinTitle(0), 0, button:MouseButton::ScrollUp   => |ctx, _| focus_stack(ctx, StackDirection::Previous)),
        btn!(WinTitle(0), 0, button:MouseButton::ScrollDown => |ctx, _| focus_stack(ctx, StackDirection::Next)),
        btn!(WinTitle(0), SHIFT, button:MouseButton::ScrollUp => |ctx, _| {
            if let Some(win) = crate::client::selected_window(ctx) {
                push_up(ctx, win)
            }
        }),
        btn!(WinTitle(0), SHIFT, button:MouseButton::ScrollDown => |ctx, _| {
            if let Some(win) = crate::client::selected_window(ctx) {
                push_down(ctx, win)
            }
        }),
        btn!(WinTitle(0), CONTROL, button:MouseButton::ScrollUp => |ctx, _| {
            if let Some(win) = crate::client::selected_window(ctx) {
                up_scale_client(ctx, win)
            }
        }),
        btn!(WinTitle(0), CONTROL, button:MouseButton::ScrollDown => |ctx, _| {
            if let Some(win) = crate::client::selected_window(ctx) {
                down_scale_client(ctx, win)
            }
        }),
        btn!(StatusText, 0,     button:MouseButton::Left        => |ctx, _| spawn(ctx, Cmd::Panther)),
        btn!(StatusText, 0,     button:MouseButton::Middle      => |ctx, _| spawn(ctx, Cmd::Term)),
        btn!(StatusText, 0,     button:MouseButton::Right       => |ctx, _| spawn(ctx, Cmd::CaretInstantSwitch)),
        btn!(StatusText, 0,     button:MouseButton::ScrollUp    => |ctx, _| spawn(ctx, Cmd::UpVol)),
        btn!(StatusText, 0,     button:MouseButton::ScrollDown  => |ctx, _| spawn(ctx, Cmd::DownVol)),
        btn!(StatusText, MODKEY, button:MouseButton::Left       => |ctx, _| spawn(ctx, Cmd::InstantSettings)),
        btn!(StatusText, MODKEY, button:MouseButton::Middle     => |ctx, _| spawn(ctx, Cmd::MuteVol)),
        btn!(StatusText, MODKEY, button:MouseButton::Right      => |ctx, _| spawn(ctx, Cmd::Spoticli)),
        btn!(StatusText, MODKEY, button:MouseButton::ScrollUp   => |ctx, _| spawn(ctx, Cmd::UpBright)),
        btn!(StatusText, MODKEY, button:MouseButton::ScrollDown => |ctx, _| spawn(ctx, Cmd::DownBright)),
        btn!(StatusText, MS, button:MouseButton::Left           => |ctx, _| spawn(ctx, Cmd::PavuControl)),
        btn!(StatusText, MC, button:MouseButton::Left           => |ctx, _| spawn(ctx, Cmd::Notify)),
        // ── Tag bar ───────────────────────────────────────────────────────
        btn!(Tag(0), 0, button:MouseButton::Left => |ctx, _| drag_tag(ctx)),
        // Right-click: the exact tag index is in the BarPosition — toggle it
        // in/out of the current view, unless it is the only visible tag.
        btn!(Tag(0), 0, button:MouseButton::Right => |ctx, pos| {
            if let BarPosition::Tag(idx) = pos {
                toggle_view_tag(ctx, idx);
            }
        }),
        btn!(Tag(0), 0, button:MouseButton::ScrollUp   => |ctx, _| crate::tags::view::scroll_view(ctx, Direction::Left)),
        btn!(Tag(0), 0, button:MouseButton::ScrollDown => |ctx, _| crate::tags::view::scroll_view(ctx, Direction::Right)),
        btn!(Tag(0), MODKEY, button:MouseButton::Left  => |ctx, _| {
            if let Some(win) = crate::client::selected_window(ctx) {
                set_client_tag(ctx, win, TagMask::ALL_BITS)
            }
        }),
        btn!(Tag(0), MODKEY, button:MouseButton::Right => |ctx, _| {
            if let Some(win) = crate::client::selected_window(ctx) {
                toggle_tag(ctx, win, TagMask::ALL_BITS)
            }
        }),
        btn!(Tag(0), MOD1, button:MouseButton::Left => |ctx, _| {
            if let Some(win) = crate::client::selected_window(ctx) {
                follow_tag(ctx, win, TagMask::ALL_BITS)
            }
        }),
        btn!(Tag(0), MODKEY, button:MouseButton::ScrollUp   => |ctx, _| shift_view(ctx, Direction::Left)),
        btn!(Tag(0), MODKEY, button:MouseButton::ScrollDown => |ctx, _| shift_view(ctx, Direction::Right)),
        // ── Root window ───────────────────────────────────────────────────
        btn!(Root, 0,     button:MouseButton::Left        => |ctx, _| spawn(ctx, Cmd::Panther)),
        btn!(Root, 0,     button:MouseButton::Middle      => |ctx, _| spawn(ctx, Cmd::InstantMenu)),
        btn!(Root, 0,     button:MouseButton::Right       => |ctx, _| spawn(ctx, Cmd::Smart)),
        btn!(Root, 0,     button:MouseButton::ScrollUp    => |ctx, _| hide_overlay(ctx)),
        btn!(Root, 0,     button:MouseButton::ScrollDown  => |ctx, _| show_overlay(ctx)),
        btn!(Root, MODKEY, button:MouseButton::Left       => |ctx, _| set_overlay(ctx)),
        btn!(Root, MODKEY, button:MouseButton::Right      => |ctx, _| spawn(ctx, Cmd::Notify)),
        // ── Client window ─────────────────────────────────────────────────
        btn!(ClientWin, MODKEY, button:MouseButton::Left   => |ctx, _| move_mouse(ctx)),
        btn!(ClientWin, MODKEY, button:MouseButton::Middle => |ctx, _| toggle_floating(ctx)),
        btn!(ClientWin, MODKEY, button:MouseButton::Right  => |ctx, _| resize_mouse_from_cursor(ctx)),
        btn!(ClientWin, MA,     button:MouseButton::Right  => |ctx, _| resize_mouse_from_cursor(ctx)),
        btn!(ClientWin, MS,     button:MouseButton::Right  => |ctx, _| {
            if let Some(win) = crate::client::selected_window(ctx) {
                resize_aspect_mouse(ctx, win)
            }
        }),
        // ── Close button ──────────────────────────────────────────────────
        btn!(CloseButton(0), 0, button:MouseButton::Left  => |ctx, _| {
            if let Some(win) = crate::client::selected_window(ctx) {
                kill_client(ctx, win)
            }
        }),
        btn!(CloseButton(0), 0, button:MouseButton::Right => |ctx, _| {
            if let Some(win) = crate::client::selected_window(ctx) {
                toggle_locked(ctx, win)
            }
        }),
        // ── Resize widget / other ─────────────────────────────────────────
        btn!(ResizeWidget(0), 0, button:MouseButton::Left => |ctx, _| draw_window(ctx)),
        btn!(ShutDown, 0, button:MouseButton::Left   => |ctx, _| spawn(ctx, Cmd::InstantShutdown)),
        btn!(ShutDown, 0, button:MouseButton::Middle => |ctx, _| spawn(ctx, Cmd::OsLock)),
        btn!(ShutDown, 0, button:MouseButton::Right  => |ctx, _| spawn(ctx, Cmd::Slock)),
        btn!(SideBar, 0,  button:MouseButton::Left   => |ctx, _| gesture_mouse(ctx)),
        btn!(StartMenu, 0,     button:MouseButton::Left  => |ctx, _| spawn(ctx, Cmd::StartMenu)),
        btn!(StartMenu, 0,     button:MouseButton::Right => |ctx, _| spawn(ctx, Cmd::QuickMenu)),
        btn!(StartMenu, SHIFT, button:MouseButton::Left  => |ctx, _| toggle_prefix(ctx)),
    ]
}
