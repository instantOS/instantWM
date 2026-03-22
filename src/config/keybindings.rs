//! Keyboard bindings: normal keys (`get_keys`) and prefix-mode keys (`get_desktop_keybinds`).

use std::rc::Rc;

use crate::client::{kill_client, shut_kill, toggle_fake_fullscreen, zoom};
use crate::config::commands_common::{ROFI_WINDOW_SWITCH, defaults, media, menu, scrot};
use crate::floating::{
    center_window, distribute_clients, key_resize, scratchpad_toggle, toggle_maximized,
};
use crate::floating::{create_overlay, set_overlay};
use crate::focus::{direction_focus, focus_last_client, focus_stack};
use crate::keyboard::{down_key, down_press, space_toggle, up_key, up_press};
use crate::layouts::{
    LayoutKind, cycle_layout_direction, inc_nmaster_by, set_layout, set_mfact, toggle_layout,
};
use crate::monitor::{Direction as PushDirection, reorder_client};
use crate::monitor::{focus_monitor, move_to_monitor_and_follow};
use crate::mouse::warp::warp_to_focus;
use crate::mouse::{begin_keyboard_move, draw_window, moveresize, resize_mouse_from_cursor};
use crate::tags::{
    follow_view, last_view, move_client, quit, send_to_monitor, shift_tag, shift_view,
    toggle_fullscreen_overview, toggle_overview, win_view,
};
use crate::toggles::{toggle_alt_tag, toggle_show_tags, toggle_sticky, unhide_all};
use crate::types::{Direction, Key, StackDirection, TagMask, ToggleAction};
use crate::util::spawn;

use super::keysyms::*;

pub const MODKEY: u32 = 1 << 6;
pub const CONTROL: u32 = 1 << 2;
pub const SHIFT: u32 = 1 << 0;
pub const MOD1: u32 = 1 << 3;

macro_rules! key {
    ($mods:expr, $sym:expr => $action:expr) => {
        Key {
            mod_mask: $mods,
            keysym: $sym,
            action: Rc::new($action),
        }
    };
}

use crate::types::MonitorDirection;

fn tag_keys(keysym: u32, tag_idx: usize) -> [Key; 6] {
    [
        // View: MOD+num
        key!(MODKEY, keysym => move |ctx| {
            crate::tags::view::view(ctx, TagMask::single(tag_idx + 1).unwrap())
        }),
        // Toggle view: MOD+Ctrl+num
        key!(MODKEY | CONTROL, keysym => move |ctx| {
            let mask = TagMask::single(tag_idx + 1).unwrap();
            crate::tags::view::toggle_view_ctx(ctx, mask)
        }),
        // Set client tag: MOD+Shift+num
        key!(MODKEY | SHIFT, keysym => move |ctx| {
            if let Some(win) = ctx.selected_client() {
                crate::tags::client_tags::set_client_tag_ctx(
                    ctx,
                    win,
                    TagMask::single(tag_idx + 1).unwrap(),
                )
            }
        }),
        // Follow tag: MOD+Alt+num
        key!(MODKEY | MOD1, keysym => move |ctx| {
            if let Some(win) = ctx.selected_client() {
                crate::tags::client_tags::follow_tag_ctx(
                    ctx,
                    win,
                    TagMask::single(tag_idx + 1).unwrap(),
                )
            }
        }),
        // Toggle tag: MOD+Ctrl+Shift+num
        key!(MODKEY | CONTROL | SHIFT, keysym => move |ctx| {
            if let Some(win) = ctx.selected_client() {
                crate::tags::client_tags::toggle_tag_ctx(
                    ctx,
                    win,
                    TagMask::single(tag_idx + 1).unwrap(),
                )
            }
        }),
        // Swap tags: MOD+Alt+Shift+num
        key!(MODKEY | MOD1 | SHIFT, keysym => move |ctx| {
            crate::tags::view::swap_tags_ctx(ctx, TagMask::single(tag_idx + 1).unwrap())
        }),
    ]
}

pub fn get_keys() -> Vec<Key> {
    let mut keys: Vec<Key> = vec![
        key!(MODKEY | MOD1, XK_J => |ctx| {
            if let Some(win) = ctx.selected_client() {
                key_resize(ctx, win, Direction::Down)
            }
        }),
        key!(MODKEY | MOD1, XK_K => |ctx| {
            if let Some(win) = ctx.selected_client() {
                key_resize(ctx, win, Direction::Up)
            }
        }),
        key!(MODKEY | MOD1, XK_L => |ctx| {
            if let Some(win) = ctx.selected_client() {
                key_resize(ctx, win, Direction::Right)
            }
        }),
        key!(MODKEY | MOD1, XK_H => |ctx| {
            if let Some(win) = ctx.selected_client() {
                key_resize(ctx, win, Direction::Left)
            }
        }),
        key!(MODKEY, XK_I => |ctx| inc_nmaster_by(ctx, 1)),
        key!(MODKEY, XK_D => |ctx| inc_nmaster_by(ctx, -1)),
        key!(MODKEY, XK_H => |ctx| set_mfact(ctx, -0.05)),
        key!(MODKEY, XK_L => |ctx| set_mfact(ctx, 0.05)),
        key!(MODKEY,    XK_T => |ctx| set_layout(ctx, LayoutKind::Tile)),
        key!(MODKEY,    XK_C => |ctx| set_layout(ctx, LayoutKind::Grid)),
        key!(MODKEY,    XK_F => |ctx| set_layout(ctx, LayoutKind::Floating)),
        key!(MODKEY,    XK_M => |ctx| set_layout(ctx, LayoutKind::Monocle)),
        key!(MODKEY,    XK_P => toggle_layout),
        key!(MODKEY | CONTROL,        XK_COMMA  => |ctx| cycle_layout_direction(ctx, false)),
        key!(MODKEY | CONTROL,        XK_PERIOD => |ctx| cycle_layout_direction(ctx, true)),
        key!(MODKEY, XK_J    => |ctx| focus_stack(ctx, StackDirection::Next)),
        key!(MODKEY, XK_K    => |ctx| focus_stack(ctx, StackDirection::Previous)),
        key!(MODKEY, XK_DOWN => |ctx| {
            down_key(ctx, StackDirection::Next)
        }),
        key!(MODKEY, XK_UP   => |ctx| {
            up_key(ctx, StackDirection::Previous)
        }),
        key!(MODKEY | SHIFT,     XK_DOWN => |ctx| {
            down_press(ctx)
        }),
        key!(MODKEY | SHIFT,     XK_UP   => |ctx| {
            up_press(ctx)
        }),
        key!(MODKEY | CONTROL, XK_J => |ctx| {
            if let Some(win) = ctx.selected_client() {
                reorder_client(ctx, win, PushDirection::Down)
            }
        }),
        key!(MODKEY | CONTROL, XK_K => |ctx| {
            if let Some(win) = ctx.selected_client() {
                reorder_client(ctx, win, PushDirection::Up)
            }
        }),
        key!(MODKEY | CONTROL, XK_LEFT  => |ctx| direction_focus(ctx, Direction::Left)),
        key!(MODKEY | CONTROL, XK_RIGHT => |ctx| direction_focus(ctx, Direction::Right)),
        key!(MODKEY | CONTROL, XK_UP    => |ctx| direction_focus(ctx, Direction::Up)),
        key!(MODKEY | CONTROL, XK_DOWN  => |ctx| direction_focus(ctx, Direction::Down)),
        key!(MODKEY,  XK_TAB     => last_view),
        key!(MODKEY | SHIFT,      XK_TAB     => focus_last_client),
        key!(MODKEY | MOD1,      XK_TAB     => follow_view),
        key!(MODKEY,  XK_LEFT    => |ctx| crate::tags::view::scroll_view(ctx, Direction::Left)),
        key!(MODKEY,  XK_RIGHT   => |ctx| crate::tags::view::scroll_view(ctx, Direction::Right)),
        key!(MODKEY | MOD1,      XK_LEFT    => |ctx| move_client(ctx, Direction::Left)),
        key!(MODKEY | MOD1,      XK_RIGHT   => |ctx| move_client(ctx, Direction::Right)),
        key!(MODKEY | SHIFT,      XK_LEFT    => |ctx| shift_tag(ctx, Direction::Left, 1)),
        key!(MODKEY | SHIFT,      XK_RIGHT   => |ctx| shift_tag(ctx, Direction::Right, 1)),
        key!(MODKEY | SHIFT | CONTROL,     XK_RIGHT   => |ctx| shift_view(ctx, Direction::Right)),
        key!(MODKEY | SHIFT | CONTROL,     XK_LEFT    => |ctx| shift_view(ctx, Direction::Left)),
        // View all tags (overview mode)
        key!(MODKEY,  XK_0       => |ctx| {
            crate::tags::view::view(ctx, TagMask::ALL_BITS)
        }),
        // Move client to all tags
        key!(MODKEY | SHIFT,      XK_0       => |ctx| {
            if let Some(win) = ctx.selected_client() {
                crate::tags::client_tags::set_client_tag_ctx(ctx, win, TagMask::ALL_BITS)
            }
        }),
        key!(MODKEY,  XK_O       => win_view),
        key!(MODKEY, XK_COMMA  => |ctx| focus_monitor(ctx, MonitorDirection::PREV)),
        key!(MODKEY, XK_PERIOD => |ctx| focus_monitor(ctx, MonitorDirection::NEXT)),
        key!(MODKEY | SHIFT,     XK_COMMA  => |ctx| send_to_monitor(ctx, MonitorDirection::PREV)),
        key!(MODKEY | SHIFT,     XK_PERIOD => |ctx| send_to_monitor(ctx, MonitorDirection::NEXT)),
        key!(MODKEY | MOD1,     XK_COMMA  => |ctx| move_to_monitor_and_follow(ctx, MonitorDirection::PREV)),
        key!(MODKEY | MOD1,     XK_PERIOD => |ctx| move_to_monitor_and_follow(ctx, MonitorDirection::NEXT)),
        key!(MODKEY | SHIFT,   XK_RETURN => zoom),
        key!(MODKEY | CONTROL,   XK_D      => distribute_clients),
        key!(MODKEY | SHIFT,   XK_D      => draw_window),
        key!(MODKEY | MOD1,   XK_W      => |ctx| {
            if let Some(win) = ctx.selected_client() {
                center_window(ctx, win)
            }
        }),
        key!(MODKEY | SHIFT,   XK_W      => warp_to_focus),
        key!(MODKEY | SHIFT,   XK_J      => |ctx| {
            if let Some(win) = ctx.selected_client() {
                moveresize(ctx, win, Direction::Down)
            }
        }),
        key!(MODKEY | SHIFT,   XK_K      => |ctx| {
            if let Some(win) = ctx.selected_client() {
                moveresize(ctx, win, Direction::Up)
            }
        }),
        key!(MODKEY | SHIFT,   XK_L      => |ctx| {
            if let Some(win) = ctx.selected_client() {
                moveresize(ctx, win, Direction::Right)
            }
        }),
        key!(MODKEY | SHIFT,   XK_H      => |ctx| {
            if let Some(win) = ctx.selected_client() {
                moveresize(ctx, win, Direction::Left)
            }
        }),
        key!(MODKEY | SHIFT,       XK_M      => begin_keyboard_move),
        key!(MODKEY | MOD1,   XK_M      => |ctx| resize_mouse_from_cursor(ctx, crate::types::MouseButton::Left)),
        key!(MODKEY, XK_E  => |ctx| {
            toggle_overview(ctx, TagMask::ALL_BITS)
        }),
        key!(MODKEY | SHIFT,     XK_E  => |ctx| {
            toggle_fullscreen_overview(ctx, TagMask::ALL_BITS)
        }),
        key!(MODKEY | CONTROL,     XK_E  => |ctx| spawn(ctx, &["instantskippy"])),
        key!(MODKEY, XK_W  => set_overlay),
        key!(MODKEY | CONTROL,     XK_W  => |ctx| {
            if let Some(win) = ctx.selected_client() {
                create_overlay(ctx, win)
            }
        }),
        key!(MODKEY, XK_S  => |ctx| scratchpad_toggle(ctx, None)),
        key!(MODKEY, XK_B  => crate::toggles::toggle_bar),
        key!(MODKEY | SHIFT,     XK_F  => toggle_fake_fullscreen),
        key!(MODKEY | CONTROL,     XK_F  => toggle_maximized),
        key!(MODKEY | CONTROL,     XK_S  => |ctx| {
            if let Some(win) = ctx.selected_client() {
                toggle_sticky(ctx, win)
            }
        }),
        key!(MODKEY | MOD1,     XK_S  => |ctx| toggle_alt_tag(ctx, ToggleAction::Toggle)),
        key!(MODKEY | SHIFT | MOD1,    XK_S  => |ctx| ctx.core_mut().globals_mut().behavior.toggle_animated(ToggleAction::Toggle)),
        key!(MODKEY | SHIFT | CONTROL,    XK_S  => |ctx| toggle_show_tags(ctx, ToggleAction::Toggle)),
        key!(MODKEY | SHIFT | MOD1,    XK_D  => |ctx| ctx.core_mut().globals_mut().behavior.toggle_double_draw()),
        key!(MODKEY | SHIFT,     XK_SPACE => |ctx| {
            space_toggle(ctx)
        }),
        // Keyboard layout cycling: Super+Alt+Space (same as instantwmctl keyboard next)
        key!(MODKEY | MOD1, XK_SPACE => |ctx| {
            crate::keyboard_layout::cycle_keyboard_layout(ctx, true);
        }),
        key!(MODKEY | SHIFT | CONTROL | MOD1,   XK_TAB   => |ctx| {
            crate::toggles::toggle_mode(ctx, "desktop");
        }),
        key!(MODKEY | CONTROL,     XK_R     => |ctx| ctx.request_bar_update(None)),
        key!(MODKEY | CONTROL,  XK_H => |ctx| {
            if let Some(win) = ctx.selected_client() {
                crate::client::hide(ctx, win)
            }
        }),
        key!(MODKEY | CONTROL | MOD1, XK_H => unhide_all),
        key!(MODKEY, XK_Q   => shut_kill),
        key!(MOD1,   XK_F4  => |ctx| {
            if let Some(win) = ctx.selected_client() {
                kill_client(ctx, win)
            }
        }),
        key!(MODKEY | SHIFT | CONTROL,    XK_Q   => |_| quit()),
        key!(MODKEY,  XK_F1 => |ctx| spawn(ctx, &["instanthotkeys", "gui"])),
        key!(MODKEY,  XK_F2 => |ctx| {
            crate::toggles::toggle_mode(ctx, "prefix");
        }),
        key!(MODKEY | SHIFT, XK_S          => |ctx| spawn(ctx, &["ins", "settings", "--gui"])),
        key!(MODKEY, XK_RETURN          => |ctx| spawn(ctx, &["kitty"])),
        key!(MODKEY, XK_SPACE           => |ctx| spawn(ctx, menu::SMART)),
        key!(MODKEY | CONTROL,     XK_SPACE           => |ctx| spawn(ctx, menu::RUN)),
        key!(MODKEY | SHIFT,     XK_V               => |ctx| spawn(ctx, menu::CLIP)),
        key!(MODKEY, XK_MINUS           => |ctx| spawn(ctx, menu::ST)),
        key!(MODKEY, XK_V               => |ctx| spawn(ctx, menu::QUICK)),
        key!(MODKEY, XK_N               => |ctx| spawn(ctx, defaults::FILEMANAGER)),
        key!(MODKEY, XK_R               => |ctx| spawn(ctx, defaults::TERM_FILEMANAGER)),
        key!(MODKEY, XK_Y               => |ctx| spawn(ctx, defaults::APPMENU)),
        key!(MODKEY, XK_X               => |ctx| spawn(ctx, &["iswitch"])),
        key!(MOD1,   XK_TAB             => |ctx| spawn(ctx, &["iswitch"])),
        key!(MODKEY, XK_DEAD_CIRCUMFLEX => |ctx| spawn(ctx, ROFI_WINDOW_SWITCH)),
        key!(MODKEY | CONTROL,     XK_L               => |ctx| spawn(ctx, defaults::LOCKSCREEN)),
        key!(MODKEY | SHIFT,     XK_ESCAPE          => |ctx| spawn(ctx, defaults::SYSTEMMONITOR)),
        key!(MODKEY, XK_PRINT => |ctx| spawn(ctx, scrot::S)),
        key!(MODKEY | SHIFT,     XK_PRINT => |ctx| spawn(ctx, scrot::M)),
        key!(MODKEY | CONTROL,     XK_PRINT => |ctx| spawn(ctx, scrot::C)),
        key!(MODKEY | MOD1,     XK_PRINT => |ctx| spawn(ctx, scrot::F)),
        key!(0, XF86XK_MON_BRIGHTNESS_UP   => |ctx| spawn(ctx, media::up_bright())),
        key!(0, XF86XK_MON_BRIGHTNESS_DOWN => |ctx| spawn(ctx, media::down_bright())),
        key!(0, XF86XK_AUDIO_LOWER_VOLUME  => |ctx| spawn(ctx, media::down_vol())),
        key!(0, XF86XK_AUDIO_MUTE          => |ctx| spawn(ctx, media::mute_vol())),
        key!(0, XF86XK_AUDIO_RAISE_VOLUME  => |ctx| spawn(ctx, media::up_vol())),
        key!(0, XF86XK_AUDIO_PLAY          => |ctx| spawn(ctx, &["playerctl", "play-pause"])),
        key!(0, XF86XK_AUDIO_PAUSE         => |ctx| spawn(ctx, &["playerctl", "play-pause"])),
        key!(0, XF86XK_AUDIO_NEXT          => |ctx| spawn(ctx, &["playerctl", "next"])),
        key!(0, XF86XK_AUDIO_PREV          => |ctx| spawn(ctx, &["playerctl", "previous"])),
    ];

    for tag_idx in 0..9 {
        keys.extend_from_slice(&tag_keys(XK_1 + tag_idx as u32, tag_idx));
    }

    keys
}

pub fn get_desktop_keybinds() -> Vec<Key> {
    vec![
        key!(0, XK_RETURN => |ctx| spawn(ctx, &["kitty"])),
        key!(0, XK_R      => |ctx| spawn(ctx, defaults::TERM_FILEMANAGER)),
        key!(0, XK_E      => |ctx| spawn(ctx, defaults::EDITOR)),
        key!(0, XK_N      => |ctx| spawn(ctx, defaults::FILEMANAGER)),
        key!(0, XK_SPACE  => |ctx| spawn(ctx, defaults::APPMENU)),
        key!(0, XK_Y      => |ctx| spawn(ctx, menu::SMART)),
        key!(0, XK_F      => |ctx| spawn(ctx, defaults::BROWSER)),
        key!(0, XK_TAB    => |ctx| spawn(ctx, ROFI_WINDOW_SWITCH)),
        key!(0, XK_PLUS   => |ctx| spawn(ctx, media::up_vol())),
        key!(0, XK_MINUS  => |ctx| spawn(ctx, media::down_vol())),
        key!(0, XK_H     => |ctx| crate::tags::view::scroll_view(ctx, Direction::Left)),
        key!(0, XK_L     => |ctx| crate::tags::view::scroll_view(ctx, Direction::Right)),
        key!(0, XK_LEFT  => |ctx| crate::tags::view::scroll_view(ctx, Direction::Left)),
        key!(0, XK_RIGHT => |ctx| crate::tags::view::scroll_view(ctx, Direction::Right)),
        key!(0, XK_K     => |ctx| shift_view(ctx, Direction::Right)),
        key!(0, XK_J     => |ctx| shift_view(ctx, Direction::Left)),
        key!(0, XK_UP    => |ctx| shift_view(ctx, Direction::Right)),
        key!(0, XK_DOWN  => |ctx| shift_view(ctx, Direction::Left)),
        // Type-safe tag views with clear semantics
        key!(0, XK_1 => |ctx| crate::tags::view::view(ctx, TagMask::single(1).unwrap())),
        key!(0, XK_2 => |ctx| crate::tags::view::view(ctx, TagMask::single(2).unwrap())),
        key!(0, XK_3 => |ctx| crate::tags::view::view(ctx, TagMask::single(3).unwrap())),
        key!(0, XK_4 => |ctx| crate::tags::view::view(ctx, TagMask::single(4).unwrap())),
        key!(0, XK_5 => |ctx| crate::tags::view::view(ctx, TagMask::single(5).unwrap())),
        key!(0, XK_6 => |ctx| crate::tags::view::view(ctx, TagMask::single(6).unwrap())),
        key!(0, XK_7 => |ctx| crate::tags::view::view(ctx, TagMask::single(7).unwrap())),
        key!(0, XK_8 => |ctx| crate::tags::view::view(ctx, TagMask::single(8).unwrap())),
        key!(0, XK_9 => |ctx| crate::tags::view::view(ctx, TagMask::single(9).unwrap())),
    ]
}
