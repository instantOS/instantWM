//! Keyboard bindings: normal keys (`get_keys`) and prefix-mode keys (`get_desktop_keybinds`).

use std::rc::Rc;

use super::commands::Cmd;
use crate::animation;
use crate::bar::x11::toggle_bar;
use crate::client::{kill_client, shut_kill, toggle_fake_fullscreen_x11, zoom};
use crate::floating::{center_window, distribute_clients, key_resize, temp_fullscreen};
use crate::focus::{direction_focus, focus_last_client_x11, focus_stack, warp_to_focus_x11};
use crate::keyboard::{down_key_x11, down_press_x11, space_toggle_x11, up_key_x11, up_press_x11};
use crate::layouts::{
    cycle_layout_direction, inc_nmaster_by, set_layout, set_mfact, toggle_layout, LayoutKind,
};
use crate::monitor::{focus_mon, follow_mon};
use crate::mouse::{draw_window, move_mouse, moveresize, resize_mouse_from_cursor};
use crate::overlay::{create_overlay, set_overlay};
use crate::push::{push_down, push_up};
use crate::scratchpad::{scratchpad_make, scratchpad_toggle};
use crate::tags::{
    follow_view, last_view, move_client, quit, send_to_monitor, shift_tag_by, shift_view,
    toggle_fullscreen_overview, toggle_overview, win_view,
};
use crate::toggles::{
    alt_tab_free, hide_window, redraw_win, toggle_alt_tag, toggle_animated, toggle_double_draw,
    toggle_prefix, toggle_show_tags, toggle_sticky, unhide_all,
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

use crate::tags::tag_ops;
use crate::types::{MonitorDirection, TagMask, TagSelection};

fn tag_keys(keysym: u32, tag_idx: usize) -> [Key; 6] {
    // Use type-safe TagMask - unwrap is safe here as tag_idx < 9
    let _mask = TagMask::single(tag_idx + 1).unwrap();

    [
        // View: MOD+num
        key_x11!(MODKEY, keysym => move |ctx| {
            tag_ops::view_selection(&mut ctx.core, &ctx.x11, TagSelection::Single(tag_idx + 1))
        }),
        // Toggle view: MOD+Ctrl+num
        key_x11!(MODKEY | CONTROL, keysym => move |ctx| {
            let mask = TagMask::single(tag_idx + 1).unwrap();
            tag_ops::view_selection(&mut ctx.core, &ctx.x11, TagSelection::Toggle(mask))
        }),
        // Set client tag: MOD+Shift+num
        key_x11!(MODKEY | SHIFT, keysym => move |ctx| {
            if let Some(win) = ctx.selected_client() {
                crate::tags::set_client_tag(&mut ctx.core, &ctx.x11, win, TagMask::single(tag_idx + 1).unwrap())
            }
        }),
        // Follow tag: MOD+Alt+num
        key_x11!(MODKEY | MOD1, keysym => move |ctx| {
            if let Some(win) = ctx.selected_client() {
                crate::tags::follow_tag(&mut ctx.core, &ctx.x11, win, TagMask::single(tag_idx + 1).unwrap())
            }
        }),
        // Toggle tag: MOD+Ctrl+Shift+num
        key_x11!(MODKEY | CONTROL | SHIFT, keysym => move |ctx| {
            if let Some(win) = ctx.selected_client() {
                crate::tags::toggle_tag(&mut ctx.core, &ctx.x11, win, TagMask::single(tag_idx + 1).unwrap())
            }
        }),
        // Swap tags: MOD+Alt+Shift+num
        key_x11!(MODKEY | MOD1 | SHIFT, keysym => move |ctx| {
            crate::tags::swap_tags(&mut ctx.core, &ctx.x11, TagMask::single(tag_idx + 1).unwrap())
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
        key_x11!(MODKEY, XK_DOWN => |ctx| down_key_x11(ctx, &ctx.x11, StackDirection::Next)),
        key_x11!(MODKEY, XK_UP   => |ctx| up_key_x11(ctx, &ctx.x11, StackDirection::Previous)),
        key_x11!(MS,     XK_DOWN => |ctx| down_press_x11(ctx, &ctx.x11)),
        key_x11!(MS,     XK_UP   => |ctx| up_press_x11(ctx, &ctx.x11)),
        key!(MC, XK_J => |ctx| {
            if let Some(win) = ctx.selected_client() {
                push_down(ctx, win)
            }
        }),
        key!(MC, XK_K => |ctx| {
            if let Some(win) = ctx.selected_client() {
                push_up(ctx, win)
            }
        }),
        key!(MC, XK_LEFT  => |ctx| direction_focus(ctx, Direction::Left)),
        key!(MC, XK_RIGHT => |ctx| direction_focus(ctx, Direction::Right)),
        key!(MC, XK_UP    => |ctx| direction_focus(ctx, Direction::Up)),
        key!(MC, XK_DOWN  => |ctx| direction_focus(ctx, Direction::Down)),
        key_x11!(MODKEY,  XK_TAB     => |ctx| last_view(&mut ctx.core, &ctx.x11)),
        key_x11!(MS,      XK_TAB     => |ctx| focus_last_client_x11(ctx, &ctx.x11)),
        key_x11!(MA,      XK_TAB     => |ctx| follow_view(&mut ctx.core, &ctx.x11)),
        key!(MODKEY,  XK_LEFT    => |ctx| animation::anim_scroll(ctx, Direction::Left)),
        key!(MODKEY,  XK_RIGHT   => |ctx| animation::anim_scroll(ctx, Direction::Right)),
        key_x11!(MA,      XK_LEFT    => |ctx| move_client(&mut ctx.core, &ctx.x11, Direction::Left)),
        key_x11!(MA,      XK_RIGHT   => |ctx| move_client(&mut ctx.core, &ctx.x11, Direction::Right)),
        key_x11!(MS,      XK_LEFT    => |ctx| shift_tag_by(&mut ctx.core, &ctx.x11, Direction::Left, 1)),
        key_x11!(MS,      XK_RIGHT   => |ctx| shift_tag_by(&mut ctx.core, &ctx.x11, Direction::Right, 1)),
        key_x11!(MSC,     XK_RIGHT   => |ctx| shift_view(&mut ctx.core, &ctx.x11, Direction::Right)),
        key_x11!(MSC,     XK_LEFT    => |ctx| shift_view(&mut ctx.core, &ctx.x11, Direction::Left)),
        // View all tags (overview mode)
        key_x11!(MODKEY,  XK_0       => |ctx| {
            tag_ops::view_selection(&mut ctx.core, &ctx.x11, TagSelection::All)
        }),
        // Move client to all tags
        key_x11!(MS,      XK_0       => |ctx| {
            use crate::types::TagMask;
            if let Some(win) = ctx.selected_client() {
                crate::tags::set_client_tag(&mut ctx.core, &ctx.x11, win, TagMask::ALL_BITS)
            }
        }),
        key_x11!(MODKEY,  XK_O       => |ctx| win_view(&mut ctx.core, &ctx.x11)),
        key!(MODKEY, XK_COMMA  => |ctx| focus_mon(ctx, MonitorDirection::PREV)),
        key!(MODKEY, XK_PERIOD => |ctx| focus_mon(ctx, MonitorDirection::NEXT)),
        key_x11!(MS,     XK_COMMA  => |ctx| send_to_monitor(&mut ctx.core, &ctx.x11, MonitorDirection::PREV)),
        key_x11!(MS,     XK_PERIOD => |ctx| send_to_monitor(&mut ctx.core, &ctx.x11, MonitorDirection::NEXT)),
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
        key_x11!(MS,   XK_W      => |ctx| warp_to_focus_x11(ctx, &ctx.x11)),
        key!(MS,   XK_J      => |ctx| {
            if let Some(win) = ctx.selected_client() {
                moveresize(ctx, win, Direction::Down)
            }
        }),
        key!(MS,   XK_K      => |ctx| {
            if let Some(win) = ctx.selected_client() {
                moveresize(ctx, win, Direction::Up)
            }
        }),
        key!(MS,   XK_L      => |ctx| {
            if let Some(win) = ctx.selected_client() {
                moveresize(ctx, win, Direction::Right)
            }
        }),
        key!(MS,   XK_H      => |ctx| {
            if let Some(win) = ctx.selected_client() {
                moveresize(ctx, win, Direction::Left)
            }
        }),
        key!(MS,   XK_M      => |ctx| move_mouse(ctx, crate::types::MouseButton::Left)),
        key!(MA,   XK_M      => |ctx| resize_mouse_from_cursor(ctx, crate::types::MouseButton::Left)),
        key_x11!(MODKEY, XK_E  => |ctx| {
            use crate::types::TagMask;
            toggle_overview(&mut ctx.core, &ctx.x11, TagMask::ALL_BITS)
        }),
        key_x11!(MS,     XK_E  => |ctx| {
            use crate::types::TagMask;
            toggle_fullscreen_overview(&mut ctx.core, &ctx.x11, TagMask::ALL_BITS)
        }),
        key!(MC,     XK_E  => |ctx| spawn(ctx, Cmd::InstantSkippy)),
        key!(MODKEY, XK_W  => set_overlay),
        key!(MC,     XK_W  => |ctx| {
            if let Some(win) = ctx.selected_client() {
                create_overlay(ctx, win)
            }
        }),
        key!(MODKEY, XK_S  => |ctx| scratchpad_toggle(ctx, None)),
        key!(MS,     XK_S  => |ctx| scratchpad_make(ctx, None)),
        key_x11!(MODKEY, XK_B  => |ctx| toggle_bar(ctx, &ctx.x11)),
        key_x11!(MS,     XK_F  => |ctx| toggle_fake_fullscreen_x11(ctx, &ctx.x11)),
        key!(MC,     XK_F  => temp_fullscreen),
        key!(MC,     XK_S  => |ctx| {
            if let Some(win) = ctx.selected_client() {
                toggle_sticky(&mut ctx.core, win)
            }
        }),
        key_x11!(MA,     XK_S  => |ctx| toggle_alt_tag(ctx, &ctx.x11, ToggleAction::Toggle)),
        key!(MSA,    XK_S  => |ctx| toggle_animated(ctx, ToggleAction::Toggle)),
        key_x11!(MSC,    XK_S  => |ctx| toggle_show_tags(ctx, &ctx.x11, ToggleAction::Toggle)),
        key!(MSA,    XK_D  => toggle_double_draw),
        key_x11!(MS,     XK_SPACE => |ctx| space_toggle_x11(ctx, &ctx.x11)),
        key_x11!(MSCA,   XK_TAB   => |ctx| alt_tab_free(ctx, &ctx.x11, ToggleAction::Toggle)),
        key_x11!(MC,     XK_R     => |ctx| redraw_win(ctx, &ctx.x11)),
        key!(MC,  XK_H => |ctx| {
            if let Some(win) = ctx.selected_client() {
                hide_window(ctx, win)
            }
        }),
        key!(MCA, XK_H => |ctx| unhide_all(ctx)),
        key!(MODKEY, XK_Q   => shut_kill),
        key!(MOD1,   XK_F4  => |ctx| {
            if let Some(win) = ctx.selected_client() {
                kill_client(ctx, win)
            }
        }),
        key!(MSC,    XK_Q   => |_| quit()),
        key!(MODKEY,  XK_F1 => |ctx| spawn(ctx, Cmd::Help)),
        key_x11!(MODKEY,  XK_F2 => |ctx| toggle_prefix(ctx, &ctx.x11)),
        key!(MODKEY, XK_RETURN          => |ctx| spawn(ctx, Cmd::Term)),
        key!(MODKEY, XK_SPACE           => |ctx| spawn(ctx, Cmd::Smart)),
        key!(MC,     XK_SPACE           => |ctx| spawn(ctx, Cmd::InstantMenu)),
        key!(MS,     XK_V               => |ctx| spawn(ctx, Cmd::ClipMenu)),
        key!(MODKEY, XK_MINUS           => |ctx| spawn(ctx, Cmd::InstantMenuSt)),
        key!(MODKEY, XK_V               => |ctx| spawn(ctx, Cmd::QuickMenu)),
        key!(MODKEY, XK_A               => |ctx| spawn(ctx, Cmd::InstantAssist)),
        key!(MS,     XK_A               => |ctx| spawn(ctx, Cmd::InstantRepeat)),
        key!(MC,     XK_I               => |ctx| spawn(ctx, Cmd::InstantPacman)),
        key!(MS,     XK_I               => |ctx| spawn(ctx, Cmd::InstantShare)),
        key!(MODKEY, XK_N               => |ctx| spawn(ctx, Cmd::Nautilus)),
        key!(MODKEY, XK_R               => |ctx| spawn(ctx, Cmd::Yazi)),
        key!(MODKEY, XK_Y               => |ctx| spawn(ctx, Cmd::Panther)),
        key!(MODKEY, XK_G               => |ctx| spawn(ctx, Cmd::Notify)),
        key!(MODKEY, XK_X               => |ctx| spawn(ctx, Cmd::InstantSwitch)),
        key!(MOD1,   XK_TAB             => |ctx| spawn(ctx, Cmd::ISwitch)),
        key!(MODKEY, XK_DEAD_CIRCUMFLEX => |ctx| spawn(ctx, Cmd::CaretInstantSwitch)),
        key!(MA,     XK_F               => |ctx| spawn(ctx, Cmd::Search)),
        key!(MA,     XK_SPACE           => |ctx| spawn(ctx, Cmd::KeyLayoutSwitch)),
        key!(MCA,    XK_L               => |ctx| spawn(ctx, Cmd::LangSwitch)),
        key!(MC,     XK_L               => |ctx| spawn(ctx, Cmd::Slock)),
        key!(MSC,    XK_L               => |ctx| spawn(ctx, Cmd::OneKeyLock)),
        key!(MC,     XK_Q               => |ctx| spawn(ctx, Cmd::InstantShutdown)),
        key!(MS,     XK_ESCAPE          => |ctx| spawn(ctx, Cmd::SystemMonitor)),
        key!(MC,     XK_C               => |ctx| spawn(ctx, Cmd::ControlCenter)),
        key!(MS,     XK_P               => |ctx| spawn(ctx, Cmd::Display)),
        key!(MODKEY, XK_PRINT => |ctx| spawn(ctx, Cmd::Scrot)),
        key!(MS,     XK_PRINT => |ctx| spawn(ctx, Cmd::FScrot)),
        key!(MC,     XK_PRINT => |ctx| spawn(ctx, Cmd::ClipScrot)),
        key!(MA,     XK_PRINT => |ctx| spawn(ctx, Cmd::FClipScrot)),
        key!(0, XF86XK_MON_BRIGHTNESS_UP   => |ctx| spawn(ctx, Cmd::UpBright)),
        key!(0, XF86XK_MON_BRIGHTNESS_DOWN => |ctx| spawn(ctx, Cmd::DownBright)),
        key!(0, XF86XK_AUDIO_LOWER_VOLUME  => |ctx| spawn(ctx, Cmd::DownVol)),
        key!(0, XF86XK_AUDIO_MUTE          => |ctx| spawn(ctx, Cmd::MuteVol)),
        key!(0, XF86XK_AUDIO_RAISE_VOLUME  => |ctx| spawn(ctx, Cmd::UpVol)),
        key!(0, XF86XK_AUDIO_PLAY          => |ctx| spawn(ctx, Cmd::PlayerPause)),
        key!(0, XF86XK_AUDIO_PAUSE         => |ctx| spawn(ctx, Cmd::PlayerPause)),
        key!(0, XF86XK_AUDIO_NEXT          => |ctx| spawn(ctx, Cmd::PlayerNext)),
        key!(0, XF86XK_AUDIO_PREV          => |ctx| spawn(ctx, Cmd::PlayerPrevious)),
    ];

    for tag_idx in 0..9 {
        keys.extend_from_slice(&tag_keys(XK_1 + tag_idx as u32, tag_idx));
    }

    keys
}

pub fn get_desktop_keybinds() -> Vec<Key> {
    vec![
        key!(0, XK_RETURN => |ctx| spawn(ctx, Cmd::Term)),
        key!(0, XK_R      => |ctx| spawn(ctx, Cmd::Yazi)),
        key!(0, XK_E      => |ctx| spawn(ctx, Cmd::Editor)),
        key!(0, XK_N      => |ctx| spawn(ctx, Cmd::Nautilus)),
        key!(0, XK_SPACE  => |ctx| spawn(ctx, Cmd::Panther)),
        key!(0, XK_F      => |ctx| spawn(ctx, Cmd::Firefox)),
        key!(0, XK_A      => |ctx| spawn(ctx, Cmd::InstantAssist)),
        key!(0, XK_F1     => |ctx| spawn(ctx, Cmd::Help)),
        key!(0, XK_M      => |ctx| spawn(ctx, Cmd::Spoticli)),
        key!(0, XK_C      => |ctx| spawn(ctx, Cmd::Code)),
        key!(0, XK_Y      => |ctx| spawn(ctx, Cmd::Smart)),
        key!(0, XK_V      => |ctx| spawn(ctx, Cmd::QuickMenu)),
        key!(0, XK_TAB    => |ctx| spawn(ctx, Cmd::CaretInstantSwitch)),
        key!(0, XK_PLUS   => |ctx| spawn(ctx, Cmd::UpVol)),
        key!(0, XK_MINUS  => |ctx| spawn(ctx, Cmd::DownVol)),
        key_x11!(0, XK_H     => |ctx| crate::tags::view::scroll_view(&mut ctx.core, &ctx.x11, Direction::Left)),
        key_x11!(0, XK_L     => |ctx| crate::tags::view::scroll_view(&mut ctx.core, &ctx.x11, Direction::Right)),
        key_x11!(0, XK_LEFT  => |ctx| crate::tags::view::scroll_view(&mut ctx.core, &ctx.x11, Direction::Left)),
        key_x11!(0, XK_RIGHT => |ctx| crate::tags::view::scroll_view(&mut ctx.core, &ctx.x11, Direction::Right)),
        key_x11!(0, XK_K     => |ctx| shift_view(&mut ctx.core, &ctx.x11, Direction::Right)),
        key_x11!(0, XK_J     => |ctx| shift_view(&mut ctx.core, &ctx.x11, Direction::Left)),
        key_x11!(0, XK_UP    => |ctx| shift_view(&mut ctx.core, &ctx.x11, Direction::Right)),
        key_x11!(0, XK_DOWN  => |ctx| shift_view(&mut ctx.core, &ctx.x11, Direction::Left)),
        // Type-safe tag views with clear semantics
        key_x11!(0, XK_1 => |ctx| tag_ops::view_selection(&mut ctx.core, &ctx.x11, TagSelection::Single(1))),
        key_x11!(0, XK_2 => |ctx| tag_ops::view_selection(&mut ctx.core, &ctx.x11, TagSelection::Single(2))),
        key_x11!(0, XK_3 => |ctx| tag_ops::view_selection(&mut ctx.core, &ctx.x11, TagSelection::Single(3))),
        key_x11!(0, XK_4 => |ctx| tag_ops::view_selection(&mut ctx.core, &ctx.x11, TagSelection::Single(4))),
        key_x11!(0, XK_5 => |ctx| tag_ops::view_selection(&mut ctx.core, &ctx.x11, TagSelection::Single(5))),
        key_x11!(0, XK_6 => |ctx| tag_ops::view_selection(&mut ctx.core, &ctx.x11, TagSelection::Single(6))),
        key_x11!(0, XK_7 => |ctx| tag_ops::view_selection(&mut ctx.core, &ctx.x11, TagSelection::Single(7))),
        key_x11!(0, XK_8 => |ctx| tag_ops::view_selection(&mut ctx.core, &ctx.x11, TagSelection::Single(8))),
        key_x11!(0, XK_9 => |ctx| tag_ops::view_selection(&mut ctx.core, &ctx.x11, TagSelection::Single(9))),
    ]
}
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


