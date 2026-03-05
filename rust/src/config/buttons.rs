//! Mouse button bindings.

use std::rc::Rc;

use super::commands::Cmd;
use super::keybindings::{CONTROL, MOD1, MODKEY, SHIFT};
use crate::animation::{down_scale_client, up_scale_client};
use crate::client::{close_win, kill_client};
use crate::focus::focus_stack_x11;
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
use crate::types::{
    BarPosition, Button, Direction, MouseButton, StackDirection, TagMask, WindowId,
};
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
            action: Rc::new($action),
        }
    };
}

pub fn get_buttons() -> Vec<Button> {
    use BarPosition::*;

    vec![
        // ── Layout symbol ─────────────────────────────────────────────────
        btn!(LtSymbol, 0,      button:MouseButton::Left   => |ctx, _| cycle_layout_direction(ctx, false)),
        btn!(LtSymbol, 0,      button:MouseButton::Right  => |ctx, _| cycle_layout_direction(ctx, true)),
        btn!(LtSymbol, 0,      button:MouseButton::Middle => |ctx, _| set_layout(ctx, LayoutKind::Tile)),
        btn!(LtSymbol, MODKEY, button:MouseButton::Left   => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                create_overlay(ctx, win)
            }
        }),
        // ── Window title ──────────────────────────────────────────────────
        // Left/right title clicks pass the event coordinates so the drag
        // handlers can use them as the anchor without a redundant round-trip.
        btn!(WinTitle(WindowId(0)), 0, button:MouseButton::Left => |ctx, arg| {
            let win = if let BarPosition::WinTitle(w) = arg.pos { w }
                      else { return };
            window_title_mouse_handler(ctx, win, arg.btn, arg.rx, arg.ry)
        }),
        btn!(WinTitle(WindowId(0)), 0, button:MouseButton::Middle => |ctx, arg| {
            let win = if let BarPosition::WinTitle(w) = arg.pos { w }
                      else { return };
            close_win(ctx, win)
        }),
        btn!(WinTitle(WindowId(0)), 0, button:MouseButton::Right => |ctx, arg| {
            let win = if let BarPosition::WinTitle(w) = arg.pos { w }
                      else { return };
            window_title_mouse_handler_right(ctx, win, arg.btn, arg.rx, arg.ry)
        }),
        btn!(WinTitle(WindowId(0)), MODKEY, button:MouseButton::Left  => |ctx, _| set_overlay(ctx)),
        btn!(WinTitle(WindowId(0)), MODKEY, button:MouseButton::Right => |ctx, _| spawn(ctx, None, Cmd::Notify)),
        btn!(WinTitle(WindowId(0)), 0,     button:MouseButton::ScrollUp   => |ctx, _| focus_stack_x11(ctx, &ctx.x11, StackDirection::Previous)),
        btn!(WinTitle(WindowId(0)), 0,     button:MouseButton::ScrollDown => |ctx, _| focus_stack_x11(ctx, &ctx.x11, StackDirection::Next)),
        btn!(WinTitle(WindowId(0)), SHIFT, button:MouseButton::ScrollUp   => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                push_up(ctx, win)
            }
        }),
        btn!(WinTitle(WindowId(0)), SHIFT, button:MouseButton::ScrollDown => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                push_down(ctx, win)
            }
        }),
        btn!(WinTitle(WindowId(0)), CONTROL, button:MouseButton::ScrollUp   => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                up_scale_client(ctx, win)
            }
        }),
        btn!(WinTitle(WindowId(0)), CONTROL, button:MouseButton::ScrollDown => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                down_scale_client(ctx, win)
            }
        }),
        // ── Status text ───────────────────────────────────────────────────
        btn!(StatusText, 0,      button:MouseButton::Left        => |ctx, _| spawn(ctx, None, Cmd::Panther)),
        btn!(StatusText, 0,      button:MouseButton::Middle      => |ctx, _| spawn(ctx, None, Cmd::Term)),
        btn!(StatusText, 0,      button:MouseButton::Right       => |ctx, _| spawn(ctx, None, Cmd::CaretInstantSwitch)),
        btn!(StatusText, 0,      button:MouseButton::ScrollUp    => |ctx, _| spawn(ctx, None, Cmd::UpVol)),
        btn!(StatusText, 0,      button:MouseButton::ScrollDown  => |ctx, _| spawn(ctx, None, Cmd::DownVol)),
        btn!(StatusText, MODKEY, button:MouseButton::Left        => |ctx, _| spawn(ctx, None, Cmd::InstantSettings)),
        btn!(StatusText, MODKEY, button:MouseButton::Middle      => |ctx, _| spawn(ctx, None, Cmd::MuteVol)),
        btn!(StatusText, MODKEY, button:MouseButton::Right       => |ctx, _| spawn(ctx, None, Cmd::Spoticli)),
        btn!(StatusText, MODKEY, button:MouseButton::ScrollUp    => |ctx, _| spawn(ctx, None, Cmd::UpBright)),
        btn!(StatusText, MODKEY, button:MouseButton::ScrollDown  => |ctx, _| spawn(ctx, None, Cmd::DownBright)),
        btn!(StatusText, MS,     button:MouseButton::Left        => |ctx, _| spawn(ctx, None, Cmd::PavuControl)),
        btn!(StatusText, MC,     button:MouseButton::Left        => |ctx, _| spawn(ctx, None, Cmd::Notify)),
        // ── Tag bar ───────────────────────────────────────────────────────
        // Left-click: pass bar_pos + event coords so drag_tag needs no
        // get_root_ptr round-trip to identify the initial tag or anchor.
        btn!(Tag(0), 0, button:MouseButton::Left => |ctx, arg| {
            drag_tag(ctx, arg.pos, arg.btn, arg.rx)
        }),
        // Right-click: tag index arrives directly in pos — toggle it in/out
        // of the current view, unless it is the only visible tag.
        btn!(Tag(0), 0, button:MouseButton::Right => |ctx, arg| {
            if let BarPosition::Tag(idx) = arg.pos {
                toggle_view_tag(&mut ctx.core, &ctx.x11, idx);
            }
        }),
        btn!(Tag(0), 0,      button:MouseButton::ScrollUp   => |ctx, _| crate::tags::view::scroll_view(&mut ctx.core, &ctx.x11, Direction::Left)),
        btn!(Tag(0), 0,      button:MouseButton::ScrollDown => |ctx, _| crate::tags::view::scroll_view(&mut ctx.core, &ctx.x11, Direction::Right)),
        btn!(Tag(0), MODKEY, button:MouseButton::Left  => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                set_client_tag(&mut ctx.core, &ctx.x11, win, TagMask::ALL_BITS)
            }
        }),
        btn!(Tag(0), MODKEY, button:MouseButton::Right => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                toggle_tag(&mut ctx.core, &ctx.x11, win, TagMask::ALL_BITS)
            }
        }),
        btn!(Tag(0), MOD1,   button:MouseButton::Left => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                follow_tag(&mut ctx.core, &ctx.x11, win, TagMask::ALL_BITS)
            }
        }),
        btn!(Tag(0), MODKEY, button:MouseButton::ScrollUp   => |ctx, _| shift_view(&mut ctx.core, &ctx.x11, Direction::Left)),
        btn!(Tag(0), MODKEY, button:MouseButton::ScrollDown => |ctx, _| shift_view(&mut ctx.core, &ctx.x11, Direction::Right)),
        // ── Root window ───────────────────────────────────────────────────
        btn!(Root, 0,      button:MouseButton::Left        => |ctx, _| spawn(ctx, None, Cmd::Panther)),
        btn!(Root, 0,      button:MouseButton::Middle      => |ctx, _| spawn(ctx, None, Cmd::InstantMenu)),
        btn!(Root, 0,      button:MouseButton::Right       => |ctx, _| spawn(ctx, None, Cmd::Smart)),
        btn!(Root, 0,      button:MouseButton::ScrollUp    => |ctx, _| hide_overlay(ctx)),
        btn!(Root, 0,      button:MouseButton::ScrollDown  => |ctx, _| show_overlay(ctx)),
        btn!(Root, MODKEY, button:MouseButton::Left        => |ctx, _| set_overlay(ctx)),
        btn!(Root, MODKEY, button:MouseButton::Right       => |ctx, _| spawn(ctx, None, Cmd::Notify)),
        // ── Client window ─────────────────────────────────────────────────
        btn!(ClientWin, MODKEY, button:MouseButton::Left   => |ctx, arg| move_mouse(ctx, arg.btn)),
        btn!(ClientWin, MODKEY, button:MouseButton::Middle => |ctx, _| toggle_floating(ctx)),
        btn!(ClientWin, MODKEY, button:MouseButton::Right  => |ctx, arg| resize_mouse_from_cursor(ctx, arg.btn)),
        btn!(ClientWin, MA,     button:MouseButton::Right  => |ctx, arg| resize_mouse_from_cursor(ctx, arg.btn)),
        btn!(ClientWin, MS,     button:MouseButton::Right  => |ctx, arg| {
            if let Some(win) = ctx.selected_client() {
                resize_aspect_mouse(ctx, win, arg.btn)
            }
        }),
        // ── Close button ──────────────────────────────────────────────────
        btn!(CloseButton(WindowId(0)), 0, button:MouseButton::Left  => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                kill_client(ctx, win)
            }
        }),
        btn!(CloseButton(WindowId(0)), 0, button:MouseButton::Right => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                toggle_locked(ctx, &ctx.x11, win)
            }
        }),
        // ── Resize widget ─────────────────────────────────────────────────
        btn!(ResizeWidget(WindowId(0)), 0, button:MouseButton::Left => |ctx, _| draw_window(ctx)),
        // ── Shutdown button ───────────────────────────────────────────────
        btn!(ShutDown, 0, button:MouseButton::Left   => |ctx, _| spawn(ctx, None, Cmd::InstantShutdown)),
        btn!(ShutDown, 0, button:MouseButton::Middle => |ctx, _| spawn(ctx, None, Cmd::OsLock)),
        btn!(ShutDown, 0, button:MouseButton::Right  => |ctx, _| spawn(ctx, None, Cmd::Slock)),
        // ── Sidebar / start menu ──────────────────────────────────────────
        btn!(SideBar, 0,       button:MouseButton::Left  => |ctx, arg| gesture_mouse(ctx, arg.btn)),
        btn!(StartMenu, 0,     button:MouseButton::Left  => |ctx, _| spawn(ctx, None, Cmd::StartMenu)),
        btn!(StartMenu, 0,     button:MouseButton::Right => |ctx, _| spawn(ctx, None, Cmd::QuickMenu)),
        btn!(StartMenu, SHIFT, button:MouseButton::Left  => |ctx, _| toggle_prefix(ctx, &ctx.x11)),
    ]
}
