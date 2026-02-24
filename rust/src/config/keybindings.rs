#![allow(deprecated)]
//! Keyboard bindings: normal keys (`get_keys`) and prefix-mode keys (`get_dkeys`).

use super::commands::Cmd;
use crate::animation::{anim_left, anim_right};
use crate::bar::toggle_bar;
use crate::client::{kill_client, shut_kill, toggle_fake_fullscreen, zoom};
use crate::floating::{center_window, distribute_clients, temp_fullscreen};
use crate::focus::{direction_focus, focus_last_client, focus_stack, warp_to_focus};
use crate::keyboard::{down_key, down_press, key_resize, space_toggle, up_key, up_press};
use crate::layouts::{cycle_layout, inc_nmaster, set_layout, set_mfact};
use crate::monitor::{focus_mon, follow_mon};
use crate::mouse::{draw_window, move_mouse, moveresize, resize_mouse_from_cursor};
use crate::overlay::{create_overlay, set_overlay};
use crate::push::{push_down, push_up};
use crate::scratchpad::{scratchpad_make, scratchpad_toggle};
use crate::tags::{
    desktop_set, follow_tag, follow_view, last_view, move_left, move_right, quit, shift_view,
    swap_tags, tag, tag_mon, tag_to_left, tag_to_right, toggle_fullscreen_overview,
    toggle_overview, toggle_tag, toggle_view, view, view_to_left, view_to_right, win_view,
};
use crate::toggles::{
    alt_tab_free, hide_window, redraw_win, toggle_alt_tag, toggle_animated, toggle_double_draw,
    toggle_prefix, toggle_show_tags, toggle_sticky, unhide_all,
};
use crate::types::{CardinalDirection, Key};
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
            action: Box::new($action),
        }
    };
}

fn tag_keys(keysym: u32, tag_idx: usize) -> [Key; 6] {
    let mask = 1u32 << tag_idx;
    [
        key!(MODKEY,                    keysym => || view(mask)),
        key!(MODKEY | CONTROL,          keysym => || toggle_view(mask)),
        key!(MODKEY | SHIFT,            keysym => || tag(mask)),
        key!(MODKEY | MOD1,             keysym => || follow_tag(mask)),
        key!(MODKEY | CONTROL | SHIFT,  keysym => || toggle_tag(mask)),
        key!(MODKEY | MOD1   | SHIFT,   keysym => || swap_tags(mask)),
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
        key!(MA, XK_J => || key_resize(CardinalDirection::Down)),
        key!(MA, XK_K => || key_resize(CardinalDirection::Up)),
        key!(MA, XK_L => || key_resize(CardinalDirection::Right)),
        key!(MA, XK_H => || key_resize(CardinalDirection::Left)),
        key!(MODKEY, XK_I => || inc_nmaster(1)),
        key!(MODKEY, XK_D => || inc_nmaster(-1)),
        key!(MODKEY, XK_H => || set_mfact(-0.05)),
        key!(MODKEY, XK_L => || set_mfact(0.05)),
        key!(MODKEY,    XK_T => || set_layout(Some(0))),
        key!(MODKEY,    XK_C => || set_layout(Some(1))),
        key!(MODKEY,    XK_F => || set_layout(Some(2))),
        key!(MODKEY,    XK_M => || set_layout(Some(3))),
        key!(MODKEY,    XK_P => || set_layout(None)),
        key!(MC,        XK_COMMA  => || cycle_layout(-1)),
        key!(MC,        XK_PERIOD => || cycle_layout(1)),
        key!(MODKEY, XK_J    => || focus_stack(1)),
        key!(MODKEY, XK_K    => || focus_stack(-1)),
        key!(MODKEY, XK_DOWN => || down_key(1)),
        key!(MODKEY, XK_UP   => || up_key(-1)),
        key!(MS,     XK_DOWN => || down_press()),
        key!(MS,     XK_UP   => || up_press()),
        key!(MC, XK_J => || push_down()),
        key!(MC, XK_K => || push_up()),
        key!(MC, XK_LEFT  => || direction_focus(3)),
        key!(MC, XK_RIGHT => || direction_focus(1)),
        key!(MC, XK_UP    => || direction_focus(0)),
        key!(MC, XK_DOWN  => || direction_focus(2)),
        key!(MODKEY,  XK_TAB     => || last_view()),
        key!(MS,      XK_TAB     => || focus_last_client()),
        key!(MA,      XK_TAB     => || follow_view()),
        key!(MODKEY,  XK_LEFT    => || anim_left()),
        key!(MODKEY,  XK_RIGHT   => || anim_right()),
        key!(MA,      XK_LEFT    => || move_left()),
        key!(MA,      XK_RIGHT   => || move_right()),
        key!(MS,      XK_LEFT    => || tag_to_left()),
        key!(MS,      XK_RIGHT   => || tag_to_right()),
        key!(MSC,     XK_RIGHT   => || shift_view(true)),
        key!(MSC,     XK_LEFT    => || shift_view(false)),
        key!(MODKEY,  XK_0       => || view(!0u32)),
        key!(MS,      XK_0       => || tag(!0u32)),
        key!(MODKEY,  XK_O       => || win_view()),
        key!(MODKEY, XK_COMMA  => || focus_mon(-1)),
        key!(MODKEY, XK_PERIOD => || focus_mon(1)),
        key!(MS,     XK_COMMA  => || tag_mon(-1)),
        key!(MS,     XK_PERIOD => || tag_mon(1)),
        key!(MA,     XK_COMMA  => || follow_mon(-1)),
        key!(MA,     XK_PERIOD => || follow_mon(1)),
        key!(MSCA,   XK_PERIOD => || desktop_set()),
        key!(MS,   XK_RETURN => || zoom()),
        key!(MC,   XK_D      => || distribute_clients()),
        key!(MS,   XK_D      => || draw_window()),
        key!(MA,   XK_W      => || center_window()),
        key!(MS,   XK_W      => || warp_to_focus()),
        key!(MS,   XK_J      => || moveresize(CardinalDirection::Down)),
        key!(MS,   XK_K      => || moveresize(CardinalDirection::Up)),
        key!(MS,   XK_L      => || moveresize(CardinalDirection::Right)),
        key!(MS,   XK_H      => || moveresize(CardinalDirection::Left)),
        key!(MS,   XK_M      => || move_mouse()),
        key!(MA,   XK_M      => || resize_mouse_from_cursor()),
        key!(MODKEY, XK_E  => || toggle_overview(!0u32)),
        key!(MS,     XK_E  => || toggle_fullscreen_overview(!0u32)),
        key!(MC,     XK_E  => || spawn(Cmd::InstantSkippy)),
        key!(MODKEY, XK_W  => || set_overlay()),
        key!(MC,     XK_W  => || create_overlay()),
        key!(MODKEY, XK_S  => || scratchpad_toggle(None)),
        key!(MS,     XK_S  => || scratchpad_make(None)),
        key!(MODKEY, XK_B  => || toggle_bar()),
        key!(MS,     XK_F  => || toggle_fake_fullscreen()),
        key!(MC,     XK_F  => || temp_fullscreen()),
        key!(MC,     XK_S  => || toggle_sticky()),
        key!(MA,     XK_S  => || toggle_alt_tag(2)),
        key!(MSA,    XK_S  => || toggle_animated(2)),
        key!(MSC,    XK_S  => || toggle_show_tags(2)),
        key!(MSA,    XK_D  => || toggle_double_draw()),
        key!(MS,     XK_SPACE => || space_toggle()),
        key!(MSCA,   XK_TAB   => || alt_tab_free()),
        key!(MC,     XK_R     => || redraw_win()),
        key!(MC,  XK_H => || hide_window()),
        key!(MCA, XK_H => || unhide_all()),
        key!(MODKEY, XK_Q   => || shut_kill()),
        key!(MOD1,   XK_F4  => || kill_client()),
        key!(MSC,    XK_Q   => || quit()),
        key!(MODKEY,  XK_F1 => || spawn(Cmd::Help)),
        key!(MODKEY,  XK_F2 => || toggle_prefix()),
        key!(MODKEY, XK_RETURN          => || spawn(Cmd::Term)),
        key!(MODKEY, XK_SPACE           => || spawn(Cmd::Smart)),
        key!(MC,     XK_SPACE           => || spawn(Cmd::InstantMenu)),
        key!(MS,     XK_V               => || spawn(Cmd::ClipMenu)),
        key!(MODKEY, XK_MINUS           => || spawn(Cmd::InstantMenuSt)),
        key!(MODKEY, XK_V               => || spawn(Cmd::QuickMenu)),
        key!(MODKEY, XK_A               => || spawn(Cmd::InstantAssist)),
        key!(MS,     XK_A               => || spawn(Cmd::InstantRepeat)),
        key!(MC,     XK_I               => || spawn(Cmd::InstantPacman)),
        key!(MS,     XK_I               => || spawn(Cmd::InstantShare)),
        key!(MODKEY, XK_N               => || spawn(Cmd::Nautilus)),
        key!(MODKEY, XK_R               => || spawn(Cmd::Yazi)),
        key!(MODKEY, XK_Y               => || spawn(Cmd::Panther)),
        key!(MODKEY, XK_G               => || spawn(Cmd::Notify)),
        key!(MODKEY, XK_X               => || spawn(Cmd::InstantSwitch)),
        key!(MOD1,   XK_TAB             => || spawn(Cmd::ISwitch)),
        key!(MODKEY, XK_DEAD_CIRCUMFLEX => || spawn(Cmd::CaretInstantSwitch)),
        key!(MA,     XK_F               => || spawn(Cmd::Search)),
        key!(MA,     XK_SPACE           => || spawn(Cmd::KeyLayoutSwitch)),
        key!(MCA,    XK_L               => || spawn(Cmd::LangSwitch)),
        key!(MC,     XK_L               => || spawn(Cmd::Slock)),
        key!(MSC,    XK_L               => || spawn(Cmd::OneKeyLock)),
        key!(MC,     XK_Q               => || spawn(Cmd::InstantShutdown)),
        key!(MS,     XK_ESCAPE          => || spawn(Cmd::SystemMonitor)),
        key!(MC,     XK_C               => || spawn(Cmd::ControlCenter)),
        key!(MS,     XK_P               => || spawn(Cmd::Display)),
        key!(MODKEY, XK_PRINT => || spawn(Cmd::Scrot)),
        key!(MS,     XK_PRINT => || spawn(Cmd::FScrot)),
        key!(MC,     XK_PRINT => || spawn(Cmd::ClipScrot)),
        key!(MA,     XK_PRINT => || spawn(Cmd::FClipScrot)),
        key!(0, XF86XK_MON_BRIGHTNESS_UP   => || spawn(Cmd::UpBright)),
        key!(0, XF86XK_MON_BRIGHTNESS_DOWN => || spawn(Cmd::DownBright)),
        key!(0, XF86XK_AUDIO_LOWER_VOLUME  => || spawn(Cmd::DownVol)),
        key!(0, XF86XK_AUDIO_MUTE          => || spawn(Cmd::MuteVol)),
        key!(0, XF86XK_AUDIO_RAISE_VOLUME  => || spawn(Cmd::UpVol)),
        key!(0, XF86XK_AUDIO_PLAY          => || spawn(Cmd::PlayerPause)),
        key!(0, XF86XK_AUDIO_PAUSE         => || spawn(Cmd::PlayerPause)),
        key!(0, XF86XK_AUDIO_NEXT          => || spawn(Cmd::PlayerNext)),
        key!(0, XF86XK_AUDIO_PREV          => || spawn(Cmd::PlayerPrevious)),
    ];

    for tag_idx in 0..9 {
        keys.extend_from_slice(&tag_keys(XK_1 + tag_idx as u32, tag_idx));
    }

    keys
}

pub fn get_dkeys() -> Vec<Key> {
    vec![
        key!(0, XK_RETURN => || spawn(Cmd::Term)),
        key!(0, XK_R      => || spawn(Cmd::Yazi)),
        key!(0, XK_E      => || spawn(Cmd::Editor)),
        key!(0, XK_N      => || spawn(Cmd::Nautilus)),
        key!(0, XK_SPACE  => || spawn(Cmd::Panther)),
        key!(0, XK_F      => || spawn(Cmd::Firefox)),
        key!(0, XK_A      => || spawn(Cmd::InstantAssist)),
        key!(0, XK_F1     => || spawn(Cmd::Help)),
        key!(0, XK_M      => || spawn(Cmd::Spoticli)),
        key!(0, XK_C      => || spawn(Cmd::Code)),
        key!(0, XK_Y      => || spawn(Cmd::Smart)),
        key!(0, XK_V      => || spawn(Cmd::QuickMenu)),
        key!(0, XK_TAB    => || spawn(Cmd::CaretInstantSwitch)),
        key!(0, XK_PLUS   => || spawn(Cmd::UpVol)),
        key!(0, XK_MINUS  => || spawn(Cmd::DownVol)),
        key!(0, XK_H     => || view_to_left()),
        key!(0, XK_L     => || view_to_right()),
        key!(0, XK_LEFT  => || view_to_left()),
        key!(0, XK_RIGHT => || view_to_right()),
        key!(0, XK_K     => || shift_view(true)),
        key!(0, XK_J     => || shift_view(false)),
        key!(0, XK_UP    => || shift_view(true)),
        key!(0, XK_DOWN  => || shift_view(false)),
        key!(0, XK_1 => || view(1 << 0)),
        key!(0, XK_2 => || view(1 << 1)),
        key!(0, XK_3 => || view(1 << 2)),
        key!(0, XK_4 => || view(1 << 3)),
        key!(0, XK_5 => || view(1 << 4)),
        key!(0, XK_6 => || view(1 << 5)),
        key!(0, XK_7 => || view(1 << 6)),
        key!(0, XK_8 => || view(1 << 7)),
        key!(0, XK_9 => || view(1 << 8)),
    ]
}
