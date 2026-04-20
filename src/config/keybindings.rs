//! Keyboard bindings: normal keys (`get_keys`) and prefix-mode keys (`get_desktop_keybinds`).

use crate::actions::{KeyAction, NamedAction, argv};
use crate::config::commands_common::{ROFI_WINDOW_SWITCH, defaults, media, menu, screenshot};
use crate::types::Key;

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
            action: $action,
        }
    };
}

fn named(action: NamedAction) -> KeyAction {
    KeyAction::Named {
        action,
        args: Vec::new(),
    }
}

fn named_args(action: NamedAction, args: &[&str]) -> KeyAction {
    KeyAction::Named {
        action,
        args: argv(args),
    }
}

fn tag_keys(keysym: u32, tag_idx: usize) -> [Key; 6] {
    [
        key!(MODKEY, keysym => KeyAction::ViewTag { tag_idx }),
        key!(MODKEY | CONTROL, keysym => KeyAction::ToggleViewTag { tag_idx }),
        key!(MODKEY | SHIFT, keysym => KeyAction::SetClientTag { tag_idx }),
        key!(MODKEY | MOD1, keysym => KeyAction::FollowClientTag { tag_idx }),
        key!(MODKEY | CONTROL | SHIFT, keysym => KeyAction::ToggleClientTag { tag_idx }),
        key!(MODKEY | MOD1 | SHIFT, keysym => KeyAction::SwapTags { tag_idx }),
    ]
}

pub fn get_keys() -> Vec<Key> {
    let mut keys: Vec<Key> = vec![
        key!(MODKEY | MOD1, XK_J => named(NamedAction::KeyResizeDown)),
        key!(MODKEY | MOD1, XK_K => named(NamedAction::KeyResizeUp)),
        key!(MODKEY | MOD1, XK_L => named(NamedAction::KeyResizeRight)),
        key!(MODKEY | MOD1, XK_H => named(NamedAction::KeyResizeLeft)),
        key!(MODKEY, XK_I => named_args(NamedAction::IncNmaster, &["1"])),
        key!(MODKEY, XK_D => named(NamedAction::DecNmaster)),
        key!(MODKEY, XK_H => named(NamedAction::MfactShrink)),
        key!(MODKEY, XK_L => named(NamedAction::MfactGrow)),
        key!(MODKEY, XK_T => named(NamedAction::LayoutTile)),
        key!(MODKEY, XK_C => named(NamedAction::LayoutGrid)),
        key!(MODKEY, XK_F => named(NamedAction::LayoutFloat)),
        key!(MODKEY, XK_M => named(NamedAction::LayoutMonocle)),
        key!(MODKEY, XK_P => named(NamedAction::ToggleLayout)),
        key!(MODKEY | CONTROL, XK_COMMA => named(NamedAction::CycleLayoutPrev)),
        key!(MODKEY | CONTROL, XK_PERIOD => named(NamedAction::CycleLayoutNext)),
        key!(MODKEY, XK_J => named(NamedAction::FocusNext)),
        key!(MODKEY, XK_K => named(NamedAction::FocusPrev)),
        key!(MODKEY, XK_DOWN => named(NamedAction::DownKey)),
        key!(MODKEY, XK_UP => named(NamedAction::UpKey)),
        key!(MODKEY | CONTROL, XK_J => named(NamedAction::PushDown)),
        key!(MODKEY | CONTROL, XK_K => named(NamedAction::PushUp)),
        key!(MODKEY | CONTROL, XK_LEFT => named(NamedAction::FocusLeft)),
        key!(MODKEY | CONTROL, XK_RIGHT => named(NamedAction::FocusRight)),
        key!(MODKEY | CONTROL, XK_UP => named(NamedAction::FocusUp)),
        key!(MODKEY | CONTROL, XK_DOWN => named(NamedAction::FocusDown)),
        key!(MODKEY, XK_TAB => named(NamedAction::LastView)),
        key!(MODKEY | SHIFT, XK_TAB => named(NamedAction::FocusLast)),
        key!(MODKEY | MOD1, XK_TAB => named(NamedAction::FollowView)),
        key!(MODKEY, XK_LEFT => named(NamedAction::ScrollLeft)),
        key!(MODKEY, XK_RIGHT => named(NamedAction::ScrollRight)),
        key!(MODKEY | MOD1, XK_LEFT => named(NamedAction::MoveClientLeft)),
        key!(MODKEY | MOD1, XK_RIGHT => named(NamedAction::MoveClientRight)),
        key!(MODKEY | SHIFT, XK_LEFT => named(NamedAction::ShiftTagLeft)),
        key!(MODKEY | SHIFT, XK_RIGHT => named(NamedAction::ShiftTagRight)),
        key!(MODKEY | SHIFT | CONTROL, XK_RIGHT => named(NamedAction::ShiftViewRight)),
        key!(MODKEY | SHIFT | CONTROL, XK_LEFT => named(NamedAction::ShiftViewLeft)),
        key!(MODKEY, XK_0 => named(NamedAction::ViewAll)),
        key!(MODKEY | SHIFT, XK_0 => named(NamedAction::TagAll)),
        key!(MODKEY, XK_O => named(NamedAction::WinView)),
        key!(MODKEY, XK_COMMA => named(NamedAction::FocusMonPrev)),
        key!(MODKEY, XK_PERIOD => named(NamedAction::FocusMonNext)),
        key!(MODKEY | MOD1, XK_COMMA => named(NamedAction::FollowMonPrev)),
        key!(MODKEY | MOD1, XK_PERIOD => named(NamedAction::FollowMonNext)),
        key!(MODKEY | SHIFT, XK_RETURN => named(NamedAction::Zoom)),
        key!(MODKEY | SHIFT, XK_SPACE => named(NamedAction::ToggleFloating)),
        key!(MODKEY | CONTROL, XK_D => named(NamedAction::DistributeClients)),
        key!(MODKEY | SHIFT, XK_D => named(NamedAction::DrawWindow)),
        key!(MODKEY | MOD1, XK_W => named(NamedAction::CenterWindow)),
        key!(MODKEY | SHIFT, XK_M => named(NamedAction::BeginKeyboardMove)),
        key!(MODKEY, XK_E => named(NamedAction::ToggleOverview)),
        key!(MODKEY | SHIFT, XK_E => named(NamedAction::CancelOverview)),
        key!(MODKEY, XK_W => named(NamedAction::EdgeScratchpadToggle)),
        key!(MODKEY | CONTROL, XK_W => named(NamedAction::EdgeScratchpadCreate)),
        key!(MODKEY | CONTROL, XK_UP => named(NamedAction::EdgeScratchpadDirectionUp)),
        key!(MODKEY | CONTROL, XK_DOWN => named(NamedAction::EdgeScratchpadDirectionDown)),
        key!(MODKEY | CONTROL, XK_LEFT => named(NamedAction::EdgeScratchpadDirectionLeft)),
        key!(MODKEY | CONTROL, XK_RIGHT => named(NamedAction::EdgeScratchpadDirectionRight)),
        key!(MODKEY, XK_S => named(NamedAction::ScratchpadToggle)),
        key!(MODKEY, XK_B => named(NamedAction::ToggleBar)),
        key!(MODKEY | CONTROL, XK_F => named(NamedAction::ToggleMaximized)),
        key!(MODKEY | CONTROL, XK_S => named(NamedAction::ToggleSticky)),
        key!(MODKEY | MOD1, XK_S => named(NamedAction::ToggleAltTag)),
        key!(MODKEY | SHIFT | MOD1, XK_S => named(NamedAction::ToggleAnimated)),
        key!(MODKEY | SHIFT | CONTROL, XK_S => named(NamedAction::ToggleShowTags)),
        key!(MODKEY | SHIFT | MOD1, XK_D => named(NamedAction::ToggleDoubleDraw)),
        key!(MODKEY | MOD1, XK_SPACE => named(NamedAction::NextKeyboardLayout)),
        key!(MODKEY | SHIFT | CONTROL | MOD1, XK_TAB => named_args(NamedAction::ModeToggle, &["desktop"])),
        key!(MODKEY | CONTROL, XK_H => named(NamedAction::Hide)),
        key!(MODKEY | CONTROL | MOD1, XK_H => named(NamedAction::UnhideAll)),
        key!(MODKEY, XK_Q => named(NamedAction::ShutKill)),
        key!(MOD1, XK_F4 => named(NamedAction::Kill)),
        key!(MODKEY | SHIFT | CONTROL, XK_Q => named(NamedAction::Quit)),
        key!(MODKEY, XK_F2 => named(NamedAction::TogglePrefix)),
        key!(MODKEY, XK_RETURN => named_args(NamedAction::Spawn, &["kitty"])),
        key!(MODKEY, XK_SPACE => named_args(NamedAction::Spawn, menu::SMART)),
        key!(MODKEY | CONTROL, XK_SPACE => named_args(NamedAction::Spawn, menu::RUN)),
        key!(MODKEY | SHIFT, XK_V => named_args(NamedAction::Spawn, menu::CLIP)),
        key!(MODKEY, XK_MINUS => named_args(NamedAction::Spawn, menu::ST)),
        key!(MODKEY, XK_V => named_args(NamedAction::Spawn, menu::QUICK)),
        key!(MODKEY, XK_N => named_args(NamedAction::Spawn, defaults::FILEMANAGER)),
        key!(MODKEY, XK_R => named_args(NamedAction::Spawn, defaults::TERM_FILEMANAGER)),
        key!(MODKEY, XK_Y => named_args(NamedAction::Spawn, defaults::APPMENU)),
        key!(MODKEY, XK_X => named_args(NamedAction::Spawn, &["iswitch"])),
        key!(MODKEY, XK_A => named_args(NamedAction::Spawn, &["ins", "assist"])),
        key!(MOD1, XK_TAB => named_args(NamedAction::Spawn, &["iswitch"])),
        key!(MODKEY, XK_DEAD_CIRCUMFLEX => named_args(NamedAction::Spawn, ROFI_WINDOW_SWITCH)),
        key!(MODKEY | CONTROL, XK_L => named_args(NamedAction::Spawn, defaults::LOCKSCREEN)),
        key!(MODKEY | SHIFT, XK_ESCAPE => named_args(NamedAction::Spawn, defaults::SYSTEMMONITOR)),
        key!(MODKEY, XK_PRINT => named_args(NamedAction::Spawn, screenshot::AREA)),
        key!(MODKEY | SHIFT, XK_PRINT => named_args(NamedAction::Spawn, screenshot::FULL)),
        key!(MODKEY | CONTROL, XK_PRINT => named_args(NamedAction::Spawn, screenshot::CLIPBOARD)),
        key!(MODKEY | MOD1, XK_PRINT => named_args(NamedAction::Spawn, screenshot::FULL_CLIPBOARD)),
        key!(0, XF86XK_MON_BRIGHTNESS_UP => named_args(NamedAction::Spawn, media::up_bright())),
        key!(0, XF86XK_MON_BRIGHTNESS_DOWN => named_args(NamedAction::Spawn, media::down_bright())),
        key!(0, XF86XK_AUDIO_LOWER_VOLUME => named_args(NamedAction::Spawn, media::down_vol())),
        key!(0, XF86XK_AUDIO_MUTE => named_args(NamedAction::Spawn, media::mute_vol())),
        key!(0, XF86XK_AUDIO_RAISE_VOLUME => named_args(NamedAction::Spawn, media::up_vol())),
        key!(0, XF86XK_AUDIO_PLAY => named_args(NamedAction::Spawn, &["playerctl", "play-pause"])),
        key!(0, XF86XK_AUDIO_PAUSE => named_args(NamedAction::Spawn, &["playerctl", "play-pause"])),
        key!(0, XF86XK_AUDIO_NEXT => named_args(NamedAction::Spawn, &["playerctl", "next"])),
        key!(0, XF86XK_AUDIO_PREV => named_args(NamedAction::Spawn, &["playerctl", "previous"])),
    ];

    for tag_idx in 0..9 {
        keys.extend_from_slice(&tag_keys(XK_1 + tag_idx as u32, tag_idx));
    }

    keys
}

pub fn get_desktop_keybinds() -> Vec<Key> {
    vec![
        key!(0, XK_RETURN => named_args(NamedAction::Spawn, &["kitty"])),
        key!(0, XK_R => named_args(NamedAction::Spawn, defaults::TERM_FILEMANAGER)),
        key!(0, XK_E => named_args(NamedAction::Spawn, defaults::EDITOR)),
        key!(0, XK_N => named_args(NamedAction::Spawn, defaults::FILEMANAGER)),
        key!(0, XK_SPACE => named_args(NamedAction::Spawn, defaults::APPMENU)),
        key!(0, XK_Y => named_args(NamedAction::Spawn, menu::SMART)),
        key!(0, XK_F => named_args(NamedAction::Spawn, defaults::BROWSER)),
        key!(0, XK_TAB => named_args(NamedAction::Spawn, ROFI_WINDOW_SWITCH)),
        key!(0, XK_PLUS => named_args(NamedAction::Spawn, media::up_vol())),
        key!(0, XK_MINUS => named_args(NamedAction::Spawn, media::down_vol())),
        key!(0, XK_H => named(NamedAction::ScrollLeft)),
        key!(0, XK_L => named(NamedAction::ScrollRight)),
        key!(0, XK_LEFT => named(NamedAction::ScrollLeft)),
        key!(0, XK_RIGHT => named(NamedAction::ScrollRight)),
        key!(0, XK_K => named(NamedAction::ShiftViewRight)),
        key!(0, XK_J => named(NamedAction::ShiftViewLeft)),
        key!(0, XK_UP => named(NamedAction::ShiftViewRight)),
        key!(0, XK_DOWN => named(NamedAction::ShiftViewLeft)),
        key!(0, XK_1 => KeyAction::ViewTag { tag_idx: 0 }),
        key!(0, XK_2 => KeyAction::ViewTag { tag_idx: 1 }),
        key!(0, XK_3 => KeyAction::ViewTag { tag_idx: 2 }),
        key!(0, XK_4 => KeyAction::ViewTag { tag_idx: 3 }),
        key!(0, XK_5 => KeyAction::ViewTag { tag_idx: 4 }),
        key!(0, XK_6 => KeyAction::ViewTag { tag_idx: 5 }),
        key!(0, XK_7 => KeyAction::ViewTag { tag_idx: 6 }),
        key!(0, XK_8 => KeyAction::ViewTag { tag_idx: 7 }),
        key!(0, XK_9 => KeyAction::ViewTag { tag_idx: 8 }),
    ]
}
