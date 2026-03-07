//! Mouse button bindings.

use std::rc::Rc;

use super::keybindings::{CONTROL, MOD1, MODKEY, SHIFT};
use crate::client::{close_win, kill_client};
use crate::config::commands_common::{defaults, media, menu, ROFI_WINDOW_SWITCH};
use crate::focus::focus_stack;
use crate::layouts::{cycle_layout_direction, set_layout, LayoutKind};

use crate::floating::toggle_floating;
use crate::mouse::{
    drag_tag, draw_window, gesture_mouse, move_mouse, resize_aspect_mouse,
    resize_mouse_from_cursor, window_title_mouse_handler, window_title_mouse_handler_right,
};
use crate::overlay::{create_overlay, hide_overlay, set_overlay, show_overlay};
use crate::push::{push, Direction as PushDirection};
use crate::tags::view::toggle_view_tag;
use crate::tags::{follow_tag_ctx, set_client_tag_ctx, shift_view, toggle_tag_ctx};
use crate::toggles::{toggle_locked, toggle_prefix};
use crate::types::{
    BarPosition, Button, Direction, MouseButton, StackDirection, TagMask, WindowId,
};
use crate::util::spawn;

const MS: u32 = MODKEY | SHIFT;
const MC: u32 = MODKEY | CONTROL;
const MA: u32 = MODKEY | MOD1;

macro_rules! btn_x11 {
    ($target:expr, $mods:expr, button:$button:expr => |$ctx:ident, $arg:ident| $action:expr) => {
        Button {
            target: $target,
            mask: $mods,
            button: $button,
            action: Rc::new(|$ctx, $arg| {
                if let crate::contexts::WmCtx::X11(ref mut ctx_x11) = $ctx {
                    let $ctx = ctx_x11;
                    $action
                }
            }),
        }
    };
    ($target:expr, $mods:expr, button:$button:expr => |$ctx:ident, _| $action:expr) => {
        Button {
            target: $target,
            mask: $mods,
            button: $button,
            action: Rc::new(|$ctx, _| {
                if let crate::contexts::WmCtx::X11(ref mut ctx_x11) = $ctx {
                    let $ctx = ctx_x11;
                    $action
                }
            }),
        }
    };
}

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
            match ctx {
                crate::contexts::WmCtx::X11(ctx_x11) => {
                    window_title_mouse_handler(ctx_x11, win, arg.btn, arg.rx, arg.ry)
                }
                crate::contexts::WmCtx::Wayland(_) => {
                    crate::mouse::drag::title_drag_begin(ctx, win, arg.btn, arg.rx, arg.ry, false);
                }
            }
        }),
        btn!(WinTitle(WindowId(0)), 0, button:MouseButton::Middle => |ctx, arg| {
            let win = if let BarPosition::WinTitle(w) = arg.pos { w }
                      else { return };
            close_win(ctx, win)
        }),
        btn!(WinTitle(WindowId(0)), 0, button:MouseButton::Right => |ctx, arg| {
            let win = if let BarPosition::WinTitle(w) = arg.pos { w }
                      else { return };
            match ctx {
                crate::contexts::WmCtx::X11(ctx_x11) => {
                    window_title_mouse_handler_right(ctx_x11, win, arg.btn, arg.rx, arg.ry)
                }
                crate::contexts::WmCtx::Wayland(_) => {
                    crate::mouse::drag::title_drag_begin(ctx, win, arg.btn, arg.rx, arg.ry, false);
                }
            }
        }),
        btn!(WinTitle(WindowId(0)), MODKEY, button:MouseButton::Left  => |ctx, _| set_overlay(ctx)),
        btn!(WinTitle(WindowId(0)), MODKEY, button:MouseButton::Right => |ctx, _| spawn(ctx, &["instantnotify"])),
        btn!(WinTitle(WindowId(0)), 0,     button:MouseButton::ScrollUp   => |ctx, _| focus_stack(ctx, StackDirection::Previous)),
        btn!(WinTitle(WindowId(0)), 0,     button:MouseButton::ScrollDown => |ctx, _| focus_stack(ctx, StackDirection::Next)),
        btn!(WinTitle(WindowId(0)), SHIFT, button:MouseButton::ScrollUp   => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                push(ctx, win, PushDirection::Up)
            }
        }),
        btn!(WinTitle(WindowId(0)), SHIFT, button:MouseButton::ScrollDown => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                push(ctx, win, PushDirection::Down)
            }
        }),
        btn!(WinTitle(WindowId(0)), CONTROL, button:MouseButton::ScrollUp   => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                crate::client::geometry::scale_client(ctx, win, 110)
            }
        }),
        btn!(WinTitle(WindowId(0)), CONTROL, button:MouseButton::ScrollDown => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                crate::client::geometry::scale_client(ctx, win, 90)
            }
        }),
        // ── Status text ───────────────────────────────────────────────────
        btn!(StatusText, 0,      button:MouseButton::Left        => |ctx, _| spawn(ctx, defaults::APPMENU)),
        btn!(StatusText, 0,      button:MouseButton::Middle      => |ctx, _| spawn(ctx, &["kitty"])),
        btn!(StatusText, 0,      button:MouseButton::Right       => |ctx, _| spawn(ctx, ROFI_WINDOW_SWITCH)),
        btn!(StatusText, 0,      button:MouseButton::ScrollUp    => |ctx, _| spawn(ctx, media::up_vol())),
        btn!(StatusText, 0,      button:MouseButton::ScrollDown  => |ctx, _| spawn(ctx, media::down_vol())),
        btn!(StatusText, MODKEY, button:MouseButton::Left        => |ctx, _| spawn(ctx, &["ins", "settings", "--gui"])),
        btn!(StatusText, MODKEY, button:MouseButton::Middle      => |ctx, _| spawn(ctx, media::mute_vol())),
        btn!(StatusText, MODKEY, button:MouseButton::Right       => |ctx, _| spawn(ctx, &["spoticli", "m"])),
        btn!(StatusText, MODKEY, button:MouseButton::ScrollUp    => |ctx, _| spawn(ctx, media::up_bright())),
        btn!(StatusText, MODKEY, button:MouseButton::ScrollDown  => |ctx, _| spawn(ctx, media::down_bright())),
        btn!(StatusText, MS,     button:MouseButton::Left        => |ctx, _| spawn(ctx, &["pavucontrol"])),
        btn!(StatusText, MC,     button:MouseButton::Left        => |ctx, _| spawn(ctx, &["instantnotify"])),
        // ── Tag bar ───────────────────────────────────────────────────────
        // Left-click: pass bar_pos + event coords so drag_tag needs no
        // get_root_ptr round-trip to identify the initial tag or anchor.
        btn!(Tag(0), 0, button:MouseButton::Left => |ctx, arg| {
            match ctx {
                crate::contexts::WmCtx::X11(ctx_x11) => drag_tag(ctx_x11, arg.pos, arg.btn, arg.rx),
                crate::contexts::WmCtx::Wayland(_) => {
                    crate::mouse::drag::drag_tag_begin(ctx, arg.pos, arg.btn);
                }
            }
        }),
        // Right-click: tag index arrives directly in pos — toggle it in/out
        // of the current view, unless it is the only visible tag.
        btn!(Tag(0), 0, button:MouseButton::Right => |ctx, arg| {
            if let BarPosition::Tag(idx) = arg.pos {
                toggle_view_tag(ctx, idx);
            }
        }),
        btn!(Tag(0), 0,      button:MouseButton::ScrollUp   => |ctx, _| crate::tags::view::scroll_view(ctx, Direction::Left)),
        btn!(Tag(0), 0,      button:MouseButton::ScrollDown => |ctx, _| crate::tags::view::scroll_view(ctx, Direction::Right)),
        btn!(Tag(0), MODKEY, button:MouseButton::Left  => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                set_client_tag_ctx(ctx, win, TagMask::ALL_BITS)
            }
        }),
        btn!(Tag(0), MODKEY, button:MouseButton::Right => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                toggle_tag_ctx(ctx, win, TagMask::ALL_BITS)
            }
        }),
        btn!(Tag(0), MOD1,   button:MouseButton::Left => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                follow_tag_ctx(ctx, win, TagMask::ALL_BITS)
            }
        }),
        btn!(Tag(0), MODKEY, button:MouseButton::ScrollUp   => |ctx, _| shift_view(ctx, Direction::Left)),
        btn!(Tag(0), MODKEY, button:MouseButton::ScrollDown => |ctx, _| shift_view(ctx, Direction::Right)),
        // ── Root window ───────────────────────────────────────────────────
        btn!(Root, 0,      button:MouseButton::Left        => |ctx, _| spawn(ctx, defaults::APPMENU)),
        btn!(Root, 0,      button:MouseButton::Middle      => |ctx, _| spawn(ctx, menu::RUN)),
        btn!(Root, 0,      button:MouseButton::Right       => |ctx, _| spawn(ctx, menu::SMART)),
        btn!(Root, 0,      button:MouseButton::ScrollUp    => |ctx, _| hide_overlay(ctx)),
        btn!(Root, 0,      button:MouseButton::ScrollDown  => |ctx, _| show_overlay(ctx)),
        btn!(Root, MODKEY, button:MouseButton::Left        => |ctx, _| set_overlay(ctx)),
        btn!(Root, MODKEY, button:MouseButton::Right       => |ctx, _| spawn(ctx, &["instantnotify"])),
        // ── Client window ─────────────────────────────────────────────────
        btn!(ClientWin, MODKEY, button:MouseButton::Left => |ctx, arg| {
            match ctx {
                crate::contexts::WmCtx::X11(ctx_x11) => move_mouse(ctx_x11, arg.btn),
                crate::contexts::WmCtx::Wayland(_) => {
                    if let Some(win) = ctx.selected_client() {
                        crate::mouse::drag::title_drag_begin(ctx, win, arg.btn, arg.rx, arg.ry, false);
                    }
                }
            }
        }),
        btn!(ClientWin, MODKEY, button:MouseButton::Middle => |ctx, _| toggle_floating(ctx)),
        btn_x11!(ClientWin, MODKEY, button:MouseButton::Right  => |ctx, arg| resize_mouse_from_cursor(ctx, arg.btn)),
        btn_x11!(ClientWin, MA,     button:MouseButton::Right  => |ctx, arg| resize_mouse_from_cursor(ctx, arg.btn)),
        btn_x11!(ClientWin, MS,     button:MouseButton::Right  => |ctx, arg| {
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
                toggle_locked(ctx, win)
            }
        }),
        // ── Resize widget ─────────────────────────────────────────────────
        btn!(ResizeWidget(WindowId(0)), 0, button:MouseButton::Left => |ctx, _| draw_window(ctx)),
        // ── Shutdown button ───────────────────────────────────────────────
        btn!(ShutDown, 0, button:MouseButton::Left   => |ctx, _| spawn(ctx, &["instantshutdown"])),
        btn!(ShutDown, 0, button:MouseButton::Middle => |ctx, _| spawn(ctx, &["instantlock", "-o"])),
        btn!(ShutDown, 0, button:MouseButton::Right  => |ctx, _| spawn(ctx, &[".config/instantos/default/lockscreen"])),
        // ── Sidebar / start menu ────────────────────────────────────────────
        btn_x11!(SideBar, 0,       button:MouseButton::Left  => |ctx, arg| gesture_mouse(ctx, arg.btn)),
        btn!(StartMenu, 0,     button:MouseButton::Left  => |ctx, _| spawn(ctx, &["instantstartmenu"])),
        btn!(StartMenu, 0,     button:MouseButton::Right => |ctx, _| spawn(ctx, &["quickmenu"])),
        btn!(StartMenu, SHIFT, button:MouseButton::Left  => |ctx, _| toggle_prefix(ctx)),
    ]
}
