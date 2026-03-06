//! Keyboard bindings: normal keys (`get_keys`) and prefix-mode keys (`get_desktop_keybinds`).

use std::rc::Rc;

use crate::animation;
use crate::bar::x11::toggle_bar;
use crate::client::{kill_client, shut_kill, toggle_fake_fullscreen_x11, zoom};
use crate::config::commands_common::{defaults, media, menu, scrot, ROFI_WINDOW_SWITCH};
use crate::floating::{center_window, distribute_clients, key_resize, toggle_maximized};
use crate::focus::{direction_focus, focus_last_client, focus_stack, warp_to_focus_x11};
use crate::keyboard::{down_key_x11, down_press_x11, space_toggle_x11, up_key_x11, up_press_x11};
use crate::layouts::{
    cycle_layout_direction, inc_nmaster_by, set_layout, set_mfact, toggle_layout, LayoutKind,
};
use crate::monitor::{focus_mon, follow_mon};
use crate::mouse::{begin_keyboard_move, draw_window, moveresize, resize_mouse_from_cursor};
use crate::overlay::{create_overlay, set_overlay};
use crate::push::{push, Direction as PushDirection};
use crate::scratchpad::{scratchpad_make, scratchpad_toggle};
use crate::tags::{
    follow_view, last_view, move_client, quit, send_to_monitor, shift_tag_by, shift_view,
    toggle_fullscreen_overview, toggle_overview, win_view,
};
use crate::toggles::{
    alt_tab_free, redraw_win, toggle_alt_tag, toggle_animated, toggle_double_draw, toggle_prefix,
    toggle_show_tags, toggle_sticky, unhide_all,
};
use crate::types::{Direction, Key, StackDirection, ToggleAction};
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

macro_rules! key_x11 {
    ($mods:expr, $sym:expr => move |$ctx:ident| $action:expr) => {
        Key {
            mod_mask: $mods,
            keysym: $sym,
            action: Rc::new(move |$ctx| {
                if let crate::contexts::WmCtx::X11(ref mut ctx_x11) = $ctx {
                    let $ctx = ctx_x11;
                    $action
                }
            }),
        }
    };
    ($mods:expr, $sym:expr => |$ctx:ident| $action:expr) => {
        Key {
            mod_mask: $mods,
            keysym: $sym,
            action: Rc::new(|$ctx| {
                if let crate::contexts::WmCtx::X11(ref mut ctx_x11) = $ctx {
                    let $ctx = ctx_x11;
                    $action
                }
            }),
        }
    };
}

use crate::types::{MonitorDirection, TagMask};

fn tag_keys(keysym: u32, tag_idx: usize) -> [Key; 6] {
    // Use type-safe TagMask - unwrap is safe here as tag_idx < 9
    let _mask = TagMask::single(tag_idx + 1).unwrap();

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

const MS: u32 = MODKEY | SHIFT;
const MC: u32 = MODKEY | CONTROL;
const MA: u32 = MODKEY | MOD1;
const MCA: u32 = MODKEY | CONTROL | MOD1;
const MSC: u32 = MODKEY | SHIFT | CONTROL;
const MSA: u32 = MODKEY | SHIFT | MOD1;
const MSCA: u32 = MODKEY | SHIFT | CONTROL | MOD1;

pub fn get_keys() -> Vec<Key> {
    let mut keys: Vec<Key> = vec![
        key!(MA, XK_J => |ctx| {
            if let Some(win) = ctx.selected_client() {
                key_resize(ctx, win, Direction::Down)
            }
        }),
        key!(MA, XK_K => |ctx| {
            if let Some(win) = ctx.selected_client() {
                key_resize(ctx, win, Direction::Up)
            }
        }),
        key!(MA, XK_L => |ctx| {
            if let Some(win) = ctx.selected_client() {
                key_resize(ctx, win, Direction::Right)
            }
        }),
        key!(MA, XK_H => |ctx| {
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
        key!(MC,        XK_COMMA  => |ctx| cycle_layout_direction(ctx, false)),
        key!(MC,        XK_PERIOD => |ctx| cycle_layout_direction(ctx, true)),
        key!(MODKEY, XK_J    => |ctx| focus_stack(ctx, StackDirection::Next)),
        key!(MODKEY, XK_K    => |ctx| focus_stack(ctx, StackDirection::Previous)),
        key_x11!(MODKEY, XK_DOWN => |ctx| {
            let systray = ctx.systray.as_deref_mut();
            down_key_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, systray, StackDirection::Next)
        }),
        key_x11!(MODKEY, XK_UP   => |ctx| {
            let systray = ctx.systray.as_deref_mut();
            up_key_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, systray, StackDirection::Previous)
        }),
        key_x11!(MS,     XK_DOWN => |ctx| {
            let systray = ctx.systray.as_deref_mut();
            down_press_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, systray)
        }),
        key_x11!(MS,     XK_UP   => |ctx| {
            let systray = ctx.systray.as_deref_mut();
            up_press_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, systray)
        }),
        key!(MC, XK_J => |ctx| {
            if let Some(win) = ctx.selected_client() {
                push(ctx, win, PushDirection::Down)
            }
        }),
        key!(MC, XK_K => |ctx| {
            if let Some(win) = ctx.selected_client() {
                push(ctx, win, PushDirection::Up)
            }
        }),
        key!(MC, XK_LEFT  => |ctx| direction_focus(ctx, Direction::Left)),
        key!(MC, XK_RIGHT => |ctx| direction_focus(ctx, Direction::Right)),
        key!(MC, XK_UP    => |ctx| direction_focus(ctx, Direction::Up)),
        key!(MC, XK_DOWN  => |ctx| direction_focus(ctx, Direction::Down)),
        key_x11!(MODKEY,  XK_TAB     => |ctx| last_view(ctx)),
        key!(MS,      XK_TAB     => |ctx| focus_last_client(ctx)),
        key_x11!(MA,      XK_TAB     => |ctx| follow_view(ctx)),
        key!(MODKEY,  XK_LEFT    => |ctx| animation::anim_scroll(ctx, Direction::Left)),
        key!(MODKEY,  XK_RIGHT   => |ctx| animation::anim_scroll(ctx, Direction::Right)),
        key!(MA,      XK_LEFT    => |ctx| move_client(ctx, Direction::Left)),
        key!(MA,      XK_RIGHT   => |ctx| move_client(ctx, Direction::Right)),
        key!(MS,      XK_LEFT    => |ctx| shift_tag_by(ctx, Direction::Left, 1)),
        key!(MS,      XK_RIGHT   => |ctx| shift_tag_by(ctx, Direction::Right, 1)),
        key!(MSC,     XK_RIGHT   => |ctx| shift_view(ctx, Direction::Right)),
        key!(MSC,     XK_LEFT    => |ctx| shift_view(ctx, Direction::Left)),
        // View all tags (overview mode)
        key!(MODKEY,  XK_0       => |ctx| {
            crate::tags::view::view(ctx, TagMask::ALL_BITS)
        }),
        // Move client to all tags
        key!(MS,      XK_0       => |ctx| {
            use crate::types::TagMask;
            if let Some(win) = ctx.selected_client() {
                crate::tags::client_tags::set_client_tag_ctx(ctx, win, TagMask::ALL_BITS)
            }
        }),
        key_x11!(MODKEY,  XK_O       => |ctx| win_view(ctx)),
        key!(MODKEY, XK_COMMA  => |ctx| focus_mon(ctx, MonitorDirection::PREV)),
        key!(MODKEY, XK_PERIOD => |ctx| focus_mon(ctx, MonitorDirection::NEXT)),
        key_x11!(MS,     XK_COMMA  => |ctx| send_to_monitor(&mut ctx.core, &ctx.x11, ctx.x11_runtime, MonitorDirection::PREV)),
        key_x11!(MS,     XK_PERIOD => |ctx| send_to_monitor(&mut ctx.core, &ctx.x11, ctx.x11_runtime, MonitorDirection::NEXT)),
        key!(MA,     XK_COMMA  => |ctx| follow_mon(ctx, MonitorDirection::PREV)),
        key!(MA,     XK_PERIOD => |ctx| follow_mon(ctx, MonitorDirection::NEXT)),
        key!(MS,   XK_RETURN => zoom),
        key!(MC,   XK_D      => distribute_clients),
        key!(MS,   XK_D      => draw_window),
        key!(MA,   XK_W      => |ctx| {
            if let Some(win) = ctx.selected_client() {
                center_window(ctx, win)
            }
        }),
        key_x11!(MS,   XK_W      => |ctx| warp_to_focus_x11(&ctx.core, &ctx.x11, ctx.x11_runtime)),
        key_x11!(MS,   XK_J      => |ctx| {
            if let Some(win) = ctx.selected_client() {
                moveresize(ctx, win, Direction::Down)
            }
        }),
        key_x11!(MS,   XK_K      => |ctx| {
            if let Some(win) = ctx.selected_client() {
                moveresize(ctx, win, Direction::Up)
            }
        }),
        key_x11!(MS,   XK_L      => |ctx| {
            if let Some(win) = ctx.selected_client() {
                moveresize(ctx, win, Direction::Right)
            }
        }),
        key_x11!(MS,   XK_H      => |ctx| {
            if let Some(win) = ctx.selected_client() {
                moveresize(ctx, win, Direction::Left)
            }
        }),
        key!(MS,       XK_M      => begin_keyboard_move),
        key_x11!(MA,   XK_M      => |ctx| resize_mouse_from_cursor(ctx, crate::types::MouseButton::Left)),
        key_x11!(MODKEY, XK_E  => |ctx| {
            use crate::types::TagMask;
            toggle_overview(ctx, TagMask::ALL_BITS)
        }),
        key_x11!(MS,     XK_E  => |ctx| {
            use crate::types::TagMask;
            toggle_fullscreen_overview(ctx, TagMask::ALL_BITS)
        }),
        key!(MC,     XK_E  => |ctx| spawn(ctx, &["instantskippy"])),
        key!(MODKEY, XK_W  => set_overlay),
        key!(MC,     XK_W  => |ctx| {
            if let Some(win) = ctx.selected_client() {
                create_overlay(ctx, win)
            }
        }),
        key!(MODKEY, XK_S  => |ctx| scratchpad_toggle(ctx, None)),
        key!(MS,     XK_S  => |ctx| scratchpad_make(ctx, None)),
        key_x11!(MODKEY, XK_B  => |ctx| toggle_bar(&mut ctx.core, &ctx.x11, ctx.x11_runtime, ctx.systray.as_deref())),
        key_x11!(MS,     XK_F  => |ctx| toggle_fake_fullscreen_x11(&mut ctx.core, &ctx.x11)),
        key!(MC,     XK_F  => toggle_maximized),
        key!(MC,     XK_S  => |ctx| {
            if let Some(win) = ctx.selected_client() {
                toggle_sticky(ctx.core_mut(), win)
            }
        }),
        key!(MA,     XK_S  => |ctx| toggle_alt_tag(ctx, ToggleAction::Toggle)),
        key!(MSA,    XK_S  => |ctx| toggle_animated(ctx.core_mut(), ToggleAction::Toggle)),
        key!(MSC,    XK_S  => |ctx| toggle_show_tags(ctx, ToggleAction::Toggle)),
        key!(MSA,    XK_D  => |ctx| toggle_double_draw(ctx.core_mut())),
        key_x11!(MS,     XK_SPACE => |ctx| {
            let systray = ctx.systray.as_deref_mut();
            space_toggle_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, systray)
        }),
        key_x11!(MSCA,   XK_TAB   => |ctx| alt_tab_free(&mut ctx.core, &ctx.x11, ctx.x11_runtime, ToggleAction::Toggle)),
        key!(MC,     XK_R     => |ctx| redraw_win(ctx)),
        key!(MC,  XK_H => |ctx| {
            if let Some(win) = ctx.selected_client() {
                crate::client::hide(ctx, win)
            }
        }),
        key!(MCA, XK_H => |ctx| unhide_all(ctx)),
        key!(MODKEY, XK_Q   => |ctx| shut_kill(ctx)),
        key!(MOD1,   XK_F4  => |ctx| {
            if let Some(win) = ctx.selected_client() {
                kill_client(ctx, win)
            }
        }),
        key!(MSC,    XK_Q   => |_| quit()),
        key!(MODKEY,  XK_F1 => |ctx| spawn(ctx, &["instanthotkeys", "gui"])),
        key!(MODKEY,  XK_F2 => |ctx| toggle_prefix(ctx)),
        key!(MODKEY, XK_RETURN          => |ctx| spawn(ctx, &["kitty"])),
        key!(MODKEY, XK_SPACE           => |ctx| spawn(ctx, menu::SMART)),
        key!(MC,     XK_SPACE           => |ctx| spawn(ctx, menu::RUN)),
        key!(MS,     XK_V               => |ctx| spawn(ctx, menu::CLIP)),
        key!(MODKEY, XK_MINUS           => |ctx| spawn(ctx, menu::ST)),
        key!(MODKEY, XK_V               => |ctx| spawn(ctx, menu::QUICK)),
        key!(MODKEY, XK_N               => |ctx| spawn(ctx, defaults::FILEMANAGER)),
        key!(MODKEY, XK_R               => |ctx| spawn(ctx, defaults::TERM_FILEMANAGER)),
        key!(MODKEY, XK_Y               => |ctx| spawn(ctx, defaults::APPMENU)),
        key!(MODKEY, XK_X               => |ctx| spawn(ctx, &["iswitch"])),
        key!(MOD1,   XK_TAB             => |ctx| spawn(ctx, &["iswitch"])),
        key!(MODKEY, XK_DEAD_CIRCUMFLEX => |ctx| spawn(ctx, ROFI_WINDOW_SWITCH)),
        key!(MC,     XK_L               => |ctx| spawn(ctx, defaults::LOCKSCREEN)),
        key!(MS,     XK_ESCAPE          => |ctx| spawn(ctx, defaults::SYSTEMMONITOR)),
        key!(MODKEY, XK_PRINT => |ctx| spawn(ctx, scrot::S)),
        key!(MS,     XK_PRINT => |ctx| spawn(ctx, scrot::M)),
        key!(MC,     XK_PRINT => |ctx| spawn(ctx, scrot::C)),
        key!(MA,     XK_PRINT => |ctx| spawn(ctx, scrot::F)),
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
