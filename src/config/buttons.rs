//! Mouse button bindings.

use std::rc::Rc;

use super::keybindings::{CONTROL, MOD1, MODKEY, SHIFT};
use crate::client::{close_win, kill_client};
use crate::config::commands_common::{ROFI_WINDOW_SWITCH, defaults, media, menu};
use crate::focus::focus_stack;
use crate::layouts::{LayoutKind, cycle_layout_direction, set_layout};

use crate::floating::{create_overlay, hide_overlay, set_overlay, show_overlay, toggle_floating};
use crate::monitor::{Direction as PushDirection, reorder_client};
use crate::mouse::{
    drag_tag, draw_window, gesture_mouse, resize_aspect_mouse, resize_mouse_from_cursor,
    window_title_mouse_handler,
};
use crate::tags::view::toggle_view_tag;
use crate::tags::{follow_tag_ctx, set_client_tag_ctx, shift_view, toggle_tag_ctx};
use crate::toggles::{toggle_locked, toggle_mode};
use crate::types::{
    BarPosition, Button, Direction, MouseButton, StackDirection, TagMask, WindowId,
};
use crate::util::spawn;

const MS: u32 = MODKEY | SHIFT;
const MC: u32 = MODKEY | CONTROL;
const MA: u32 = MODKEY | MOD1;

fn tag_mask_from_pos(pos: BarPosition) -> Option<TagMask> {
    match pos {
        BarPosition::Tag(idx) => TagMask::single(idx + 1),
        _ => None,
    }
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
            window_title_mouse_handler(ctx, win, arg.btn, arg.rx, arg.ry)
        }),
        btn!(WinTitle(WindowId(0)), MODKEY, button:MouseButton::Left  => |ctx, _| set_overlay(ctx)),
        btn!(WinTitle(WindowId(0)), MODKEY, button:MouseButton::Right => |ctx, _| spawn(ctx, &["instantnotify"])),
        btn!(WinTitle(WindowId(0)), 0,     button:MouseButton::ScrollUp   => |ctx, _| focus_stack(ctx, StackDirection::Previous)),
        btn!(WinTitle(WindowId(0)), 0,     button:MouseButton::ScrollDown => |ctx, _| focus_stack(ctx, StackDirection::Next)),
        btn!(WinTitle(WindowId(0)), SHIFT, button:MouseButton::ScrollUp   => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                reorder_client(ctx, win, PushDirection::Up)
            }
        }),
        btn!(WinTitle(WindowId(0)), SHIFT, button:MouseButton::ScrollDown => |ctx, _| {
            if let Some(win) = ctx.selected_client() {
                reorder_client(ctx, win, PushDirection::Down)
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
        btn!(Tag(0), MODKEY, button:MouseButton::Left  => |ctx, arg| {
            if let Some(win) = ctx.selected_client()
                && let Some(tag_mask) = tag_mask_from_pos(arg.pos) {
                    set_client_tag_ctx(ctx, win, tag_mask)
                }
        }),
        btn!(Tag(0), MODKEY, button:MouseButton::Right => |ctx, arg| {
            if let Some(win) = ctx.selected_client()
                && let Some(tag_mask) = tag_mask_from_pos(arg.pos) {
                    toggle_tag_ctx(ctx, win, tag_mask)
                }
        }),
        btn!(Tag(0), MOD1,   button:MouseButton::Left => |ctx, arg| {
            if let Some(win) = ctx.selected_client()
                && let Some(tag_mask) = tag_mask_from_pos(arg.pos) {
                    follow_tag_ctx(ctx, win, tag_mask)
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
                crate::contexts::WmCtx::X11(ctx_x11) => {
                    crate::backend::x11::mouse::move_mouse_x11(ctx_x11, arg.btn, None)
                }
                crate::contexts::WmCtx::Wayland(_) => {
                    if let Some(win) = ctx.selected_client() {
                        crate::mouse::drag::title_drag_begin(ctx, win, arg.btn, arg.rx, arg.ry, false);
                    }
                }
            }
        }),
        btn!(ClientWin, MODKEY, button:MouseButton::Middle => |ctx, _| toggle_floating(ctx)),
        btn!(ClientWin, MODKEY, button:MouseButton::Right => |ctx, arg| resize_mouse_from_cursor(ctx, arg.btn)),
        btn!(ClientWin, MA, button:MouseButton::Right => |ctx, arg| resize_mouse_from_cursor(ctx, arg.btn)),
        btn!(ClientWin, MS, button:MouseButton::Right => |ctx, arg| {
            if let Some(win) = ctx.selected_client() {
                resize_aspect_mouse(ctx, win, arg.btn);
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
        btn!(SideBar, 0, button:MouseButton::Left => |ctx, arg| gesture_mouse(ctx, arg.btn)),
        btn!(StartMenu, 0,     button:MouseButton::Left  => |ctx, _| spawn(ctx, &["instantstartmenu"])),
        btn!(StartMenu, 0,     button:MouseButton::Right => |ctx, _| spawn(ctx, &["quickmenu"])),
        btn!(StartMenu, SHIFT, button:MouseButton::Left  => |ctx, _| toggle_mode(ctx, "prefix")),
    ]
}
