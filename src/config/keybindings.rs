//! Keyboard bindings: normal keys (`get_keys`) and prefix-mode keys (`get_desktop_keybinds`).

use crate::actions::{KeyAction, NamedAction};
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
        key!(MODKEY | MOD1, XK_J => KeyAction::named(NamedAction::KeyResizeDown)),
        key!(MODKEY | MOD1, XK_K => KeyAction::named(NamedAction::KeyResizeUp)),
        key!(MODKEY | MOD1, XK_L => KeyAction::named(NamedAction::KeyResizeRight)),
        key!(MODKEY | MOD1, XK_H => KeyAction::named(NamedAction::KeyResizeLeft)),
        key!(MODKEY, XK_PLUS => KeyAction::named(NamedAction::TreeGrow)),
        key!(MODKEY | SHIFT, XK_PLUS => KeyAction::named(NamedAction::TreeGrow)),
        // X11 resolves the shifted '+' key from its base '=' keysym.
        key!(MODKEY | SHIFT, XK_EQUAL => KeyAction::named(NamedAction::TreeGrow)),
        key!(MODKEY, XK_MINUS => KeyAction::named(NamedAction::TreeShrink)),
        key!(MODKEY | SHIFT, XK_J => KeyAction::named(NamedAction::KeyMoveDown)),
        key!(MODKEY | SHIFT, XK_K => KeyAction::named(NamedAction::KeyMoveUp)),
        key!(MODKEY | SHIFT, XK_L => KeyAction::named(NamedAction::KeyMoveRight)),
        key!(MODKEY | SHIFT, XK_H => KeyAction::named(NamedAction::KeyMoveLeft)),
        key!(MODKEY, XK_I => KeyAction::named_args(NamedAction::IncMasterCount, &["1"])),
        key!(MODKEY, XK_D => KeyAction::named(NamedAction::DecMasterCount)),
        key!(MODKEY, XK_H => KeyAction::named(NamedAction::MasterFactorShrink)),
        key!(MODKEY, XK_L => KeyAction::named(NamedAction::MasterFactorGrow)),
        key!(MODKEY, XK_T => KeyAction::named(NamedAction::EdgeScratchpadToggle)),
        key!(MODKEY, XK_C => KeyAction::named(NamedAction::LayoutGrid)),
        key!(MODKEY, XK_F => KeyAction::named(NamedAction::LayoutFloat)),
        key!(MODKEY, XK_W => KeyAction::named(NamedAction::ToggleTilingMaximized)),
        key!(MODKEY, XK_P => KeyAction::named(NamedAction::ToggleLayout)),
        key!(MODKEY | CONTROL, XK_COMMA => KeyAction::named(NamedAction::CycleLayoutPrev)),
        key!(MODKEY | CONTROL, XK_PERIOD => KeyAction::named(NamedAction::CycleLayoutNext)),
        key!(MODKEY, XK_J => KeyAction::named(NamedAction::FocusNext)),
        key!(MODKEY, XK_K => KeyAction::named(NamedAction::FocusPrev)),
        key!(MODKEY, XK_LEFT => KeyAction::named(NamedAction::FocusLeft)),
        key!(MODKEY, XK_RIGHT => KeyAction::named(NamedAction::FocusRight)),
        key!(MODKEY, XK_UP => KeyAction::named(NamedAction::FocusUp)),
        key!(MODKEY, XK_DOWN => KeyAction::named(NamedAction::FocusDown)),
        key!(MODKEY | CONTROL, XK_J => KeyAction::named(NamedAction::PushDown)),
        key!(MODKEY | CONTROL, XK_K => KeyAction::named(NamedAction::PushUp)),
        key!(MODKEY | SHIFT, XK_LEFT => KeyAction::named(NamedAction::KeyMoveLeft)),
        key!(MODKEY | SHIFT, XK_RIGHT => KeyAction::named(NamedAction::KeyMoveRight)),
        key!(MODKEY | SHIFT, XK_UP => KeyAction::named(NamedAction::KeyMoveUp)),
        key!(MODKEY | SHIFT, XK_DOWN => KeyAction::named(NamedAction::KeyMoveDown)),
        key!(MODKEY | CONTROL, XK_LEFT => KeyAction::named(NamedAction::KeyResizeLeft)),
        key!(MODKEY | CONTROL, XK_RIGHT => KeyAction::named(NamedAction::KeyResizeRight)),
        key!(MODKEY | CONTROL, XK_UP => KeyAction::named(NamedAction::KeyResizeUp)),
        key!(MODKEY | CONTROL, XK_DOWN => KeyAction::named(NamedAction::KeyResizeDown)),
        key!(MODKEY, XK_TAB => KeyAction::named(NamedAction::LastView)),
        key!(MODKEY | SHIFT, XK_TAB => KeyAction::named(NamedAction::FocusLast)),
        key!(MODKEY | MOD1, XK_TAB => KeyAction::named(NamedAction::FollowView)),
        key!(MODKEY | MOD1, XK_LEFT => KeyAction::named(NamedAction::MoveClientLeft)),
        key!(MODKEY | MOD1, XK_RIGHT => KeyAction::named(NamedAction::MoveClientRight)),
        key!(MODKEY | SHIFT | CONTROL, XK_RIGHT => KeyAction::named(NamedAction::ShiftViewRight)),
        key!(MODKEY | SHIFT | CONTROL, XK_LEFT => KeyAction::named(NamedAction::ShiftViewLeft)),
        key!(MODKEY, XK_0 => KeyAction::named(NamedAction::ViewAll)),
        key!(MODKEY | SHIFT, XK_0 => KeyAction::named(NamedAction::TagAll)),
        key!(MODKEY, XK_O => KeyAction::named(NamedAction::WinView)),
        key!(MODKEY, XK_COMMA => KeyAction::named(NamedAction::FocusMonPrev)),
        key!(MODKEY, XK_PERIOD => KeyAction::named(NamedAction::FocusMonNext)),
        key!(MODKEY | MOD1, XK_COMMA => KeyAction::named(NamedAction::FollowMonPrev)),
        key!(MODKEY | MOD1, XK_PERIOD => KeyAction::named(NamedAction::FollowMonNext)),
        key!(MODKEY | SHIFT, XK_RETURN => KeyAction::named(NamedAction::Zoom)),
        key!(MODKEY | SHIFT, XK_SPACE => KeyAction::named(NamedAction::ToggleFloating)),
        key!(MODKEY | CONTROL, XK_D => KeyAction::named(NamedAction::DistributeClients)),
        key!(MODKEY | SHIFT, XK_D => KeyAction::named(NamedAction::DrawWindow)),
        key!(MODKEY | MOD1, XK_W => KeyAction::named(NamedAction::CenterWindow)),
        key!(MODKEY, XK_M => KeyAction::named(NamedAction::BeginTreePlacement)),
        key!(MODKEY, XK_E => KeyAction::named(NamedAction::ToggleOverview)),
        key!(MODKEY | SHIFT, XK_E => KeyAction::named(NamedAction::CancelOverview)),
        key!(MODKEY | CONTROL, XK_T => KeyAction::named(NamedAction::EdgeScratchpadCreate)),
        key!(MODKEY, XK_S => KeyAction::named(NamedAction::ScratchpadToggle)),
        key!(MODKEY, XK_B => KeyAction::named(NamedAction::ToggleBar)),
        key!(MODKEY | CONTROL, XK_S => KeyAction::named(NamedAction::ToggleSticky)),
        key!(MODKEY | MOD1, XK_S => KeyAction::named(NamedAction::ToggleAltTag)),
        key!(MODKEY | SHIFT | MOD1, XK_S => KeyAction::named(NamedAction::ToggleAnimated)),
        key!(MODKEY | SHIFT | CONTROL, XK_S => KeyAction::named(NamedAction::ToggleShowTags)),
        key!(MODKEY | MOD1, XK_SPACE => KeyAction::named(NamedAction::NextKeyboardLayout)),
        key!(MODKEY | SHIFT | CONTROL | MOD1, XK_TAB => KeyAction::named_args(NamedAction::ModeToggle, &["desktop"])),
        key!(MODKEY | CONTROL, XK_H => KeyAction::named(NamedAction::Hide)),
        key!(MODKEY | CONTROL | MOD1, XK_H => KeyAction::named(NamedAction::UnhideAll)),
        key!(MODKEY, XK_Q => KeyAction::named(NamedAction::ShutKill)),
        key!(MOD1, XK_F4 => KeyAction::named(NamedAction::Kill)),
        key!(MODKEY | SHIFT | CONTROL, XK_Q => KeyAction::named(NamedAction::Quit)),
        key!(MODKEY, XK_F2 => KeyAction::named(NamedAction::TogglePrefix)),
        key!(MODKEY, XK_RETURN => KeyAction::named_args(NamedAction::Spawn, &["kitty"])),
        key!(MODKEY, XK_SPACE => KeyAction::named_args(NamedAction::Spawn, menu::SMART)),
        key!(MODKEY | CONTROL, XK_SPACE => KeyAction::named_args(NamedAction::Spawn, menu::RUN)),
        key!(MODKEY | SHIFT, XK_V => KeyAction::named_args(NamedAction::Spawn, menu::CLIP)),
        key!(MODKEY | MOD1, XK_MINUS => KeyAction::named_args(NamedAction::Spawn, menu::ST)),
        key!(MODKEY, XK_V => KeyAction::named_args(NamedAction::Spawn, menu::QUICK)),
        key!(MODKEY, XK_N => KeyAction::named_args(NamedAction::Spawn, defaults::FILEMANAGER)),
        key!(MODKEY, XK_R => KeyAction::named_args(NamedAction::Spawn, defaults::TERM_FILEMANAGER)),
        key!(MODKEY, XK_Y => KeyAction::named_args(NamedAction::Spawn, defaults::APPMENU)),
        key!(MODKEY, XK_X => KeyAction::named_args(NamedAction::Spawn, &["iswitch"])),
        key!(MODKEY, XK_A => KeyAction::named_args(NamedAction::Spawn, &["ins", "assist"])),
        key!(MOD1, XK_TAB => KeyAction::named_args(NamedAction::Spawn, &["iswitch"])),
        key!(MODKEY, XK_DEAD_CIRCUMFLEX => KeyAction::named_args(NamedAction::Spawn, ROFI_WINDOW_SWITCH)),
        key!(MODKEY | CONTROL, XK_L => KeyAction::named_args(NamedAction::Spawn, defaults::LOCKSCREEN)),
        key!(MODKEY | SHIFT, XK_ESCAPE => KeyAction::named_args(NamedAction::Spawn, defaults::SYSTEMMONITOR)),
        key!(MODKEY, XK_PRINT => KeyAction::named_args(NamedAction::Spawn, screenshot::AREA)),
        key!(MODKEY | SHIFT, XK_PRINT => KeyAction::named_args(NamedAction::Spawn, screenshot::FULL)),
        key!(MODKEY | CONTROL, XK_PRINT => KeyAction::named_args(NamedAction::Spawn, screenshot::CLIPBOARD)),
        key!(MODKEY | MOD1, XK_PRINT => KeyAction::named_args(NamedAction::Spawn, screenshot::FULL_CLIPBOARD)),
        key!(0, XF86XK_MON_BRIGHTNESS_UP => KeyAction::named_args(NamedAction::Spawn, media::up_bright())),
        key!(0, XF86XK_MON_BRIGHTNESS_DOWN => KeyAction::named_args(NamedAction::Spawn, media::down_bright())),
        key!(0, XF86XK_AUDIO_LOWER_VOLUME => KeyAction::named_args(NamedAction::Spawn, media::down_vol())),
        key!(0, XF86XK_AUDIO_MUTE => KeyAction::named_args(NamedAction::Spawn, media::mute_vol())),
        key!(0, XF86XK_AUDIO_RAISE_VOLUME => KeyAction::named_args(NamedAction::Spawn, media::up_vol())),
        key!(0, XF86XK_AUDIO_PLAY => KeyAction::named_args(NamedAction::Spawn, &["playerctl", "play-pause"])),
        key!(0, XF86XK_AUDIO_PAUSE => KeyAction::named_args(NamedAction::Spawn, &["playerctl", "play-pause"])),
        key!(0, XF86XK_AUDIO_NEXT => KeyAction::named_args(NamedAction::Spawn, &["playerctl", "next"])),
        key!(0, XF86XK_AUDIO_PREV => KeyAction::named_args(NamedAction::Spawn, &["playerctl", "previous"])),
    ];

    for tag_idx in 0..9 {
        keys.extend_from_slice(&tag_keys(XK_1 + tag_idx as u32, tag_idx));
    }

    keys
}

pub fn get_desktop_keybinds() -> Vec<Key> {
    vec![
        key!(0, XK_RETURN => KeyAction::named_args(NamedAction::Spawn, &["kitty"])),
        key!(0, XK_R => KeyAction::named_args(NamedAction::Spawn, defaults::TERM_FILEMANAGER)),
        key!(0, XK_E => KeyAction::named_args(NamedAction::Spawn, defaults::EDITOR)),
        key!(0, XK_N => KeyAction::named_args(NamedAction::Spawn, defaults::FILEMANAGER)),
        key!(0, XK_SPACE => KeyAction::named_args(NamedAction::Spawn, defaults::APPMENU)),
        key!(0, XK_Y => KeyAction::named_args(NamedAction::Spawn, menu::SMART)),
        key!(0, XK_F => KeyAction::named_args(NamedAction::Spawn, defaults::BROWSER)),
        key!(0, XK_TAB => KeyAction::named_args(NamedAction::Spawn, ROFI_WINDOW_SWITCH)),
        key!(0, XK_PLUS => KeyAction::named_args(NamedAction::Spawn, media::up_vol())),
        key!(0, XK_MINUS => KeyAction::named_args(NamedAction::Spawn, media::down_vol())),
        key!(0, XK_H => KeyAction::named(NamedAction::ScrollLeft)),
        key!(0, XK_L => KeyAction::named(NamedAction::ScrollRight)),
        key!(0, XK_LEFT => KeyAction::named(NamedAction::ScrollLeft)),
        key!(0, XK_RIGHT => KeyAction::named(NamedAction::ScrollRight)),
        key!(0, XK_K => KeyAction::named(NamedAction::ShiftViewRight)),
        key!(0, XK_J => KeyAction::named(NamedAction::ShiftViewLeft)),
        key!(0, XK_UP => KeyAction::named(NamedAction::ShiftViewRight)),
        key!(0, XK_DOWN => KeyAction::named(NamedAction::ShiftViewLeft)),
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

/// Default bindings for the compositor-owned tree placement mode. Super is
/// intentionally absent: the dispatcher ignores a still-held Super modifier
/// after the Super+M entry chord, while every binding remains configurable as
/// an ordinary named action under `[modes.placement]`.
pub fn get_tree_placement_keybinds() -> Vec<Key> {
    vec![
        key!(0, XK_LEFT => KeyAction::named(NamedAction::PlacementLeft)),
        key!(0, XK_H => KeyAction::named(NamedAction::PlacementLeft)),
        key!(0, XK_RIGHT => KeyAction::named(NamedAction::PlacementRight)),
        key!(0, XK_L => KeyAction::named(NamedAction::PlacementRight)),
        key!(0, XK_UP => KeyAction::named(NamedAction::PlacementUp)),
        key!(0, XK_K => KeyAction::named(NamedAction::PlacementUp)),
        key!(0, XK_DOWN => KeyAction::named(NamedAction::PlacementDown)),
        key!(0, XK_J => KeyAction::named(NamedAction::PlacementDown)),
        key!(SHIFT, XK_LEFT => KeyAction::named(NamedAction::PlacementSwapLeft)),
        key!(SHIFT, XK_H => KeyAction::named(NamedAction::PlacementSwapLeft)),
        key!(SHIFT, XK_RIGHT => KeyAction::named(NamedAction::PlacementSwapRight)),
        key!(SHIFT, XK_L => KeyAction::named(NamedAction::PlacementSwapRight)),
        key!(SHIFT, XK_UP => KeyAction::named(NamedAction::PlacementSwapUp)),
        key!(SHIFT, XK_K => KeyAction::named(NamedAction::PlacementSwapUp)),
        key!(SHIFT, XK_DOWN => KeyAction::named(NamedAction::PlacementSwapDown)),
        key!(SHIFT, XK_J => KeyAction::named(NamedAction::PlacementSwapDown)),
        key!(CONTROL, XK_LEFT => KeyAction::named(NamedAction::PlacementResizeLeft)),
        key!(CONTROL, XK_H => KeyAction::named(NamedAction::PlacementResizeLeft)),
        key!(CONTROL, XK_RIGHT => KeyAction::named(NamedAction::PlacementResizeRight)),
        key!(CONTROL, XK_L => KeyAction::named(NamedAction::PlacementResizeRight)),
        key!(CONTROL, XK_UP => KeyAction::named(NamedAction::PlacementResizeUp)),
        key!(CONTROL, XK_K => KeyAction::named(NamedAction::PlacementResizeUp)),
        key!(CONTROL, XK_DOWN => KeyAction::named(NamedAction::PlacementResizeDown)),
        key!(CONTROL, XK_J => KeyAction::named(NamedAction::PlacementResizeDown)),
        key!(0, XK_TAB => KeyAction::named(NamedAction::PlacementNext)),
        key!(SHIFT, XK_TAB => KeyAction::named(NamedAction::PlacementPrevious)),
        key!(0, XK_SPACE => KeyAction::named(NamedAction::PlacementCenter)),
        key!(0, XK_RETURN => KeyAction::named(NamedAction::PlacementApply)),
        key!(0, XK_ESCAPE => KeyAction::named(NamedAction::PlacementCancel)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named_action(modifiers: u32, keysym: u32) -> Option<NamedAction> {
        get_tree_placement_keybinds()
            .into_iter()
            .find(|key| key.mod_mask == modifiers && key.keysym == keysym)
            .and_then(|key| match key.action {
                KeyAction::Named { action, .. } => Some(action),
                _ => None,
            })
    }

    fn default_named_action(modifiers: u32, keysym: u32) -> Option<NamedAction> {
        get_keys()
            .into_iter()
            .find(|key| key.mod_mask == modifiers && key.keysym == keysym)
            .and_then(|key| match key.action {
                KeyAction::Named { action, .. } => Some(action),
                _ => None,
            })
    }

    #[test]
    fn presentation_and_overlay_defaults_use_direct_super_bindings() {
        assert_eq!(
            default_named_action(MODKEY, XK_T),
            Some(NamedAction::EdgeScratchpadToggle)
        );
        assert_eq!(
            default_named_action(MODKEY, XK_W),
            Some(NamedAction::ToggleTilingMaximized)
        );
        assert_eq!(default_named_action(MODKEY | CONTROL, XK_M), None);
    }

    #[test]
    fn placement_defaults_are_regular_named_actions() {
        assert_eq!(named_action(0, XK_H), Some(NamedAction::PlacementLeft));
        assert_eq!(
            named_action(SHIFT, XK_LEFT),
            Some(NamedAction::PlacementSwapLeft)
        );
        assert_eq!(
            named_action(CONTROL, XK_J),
            Some(NamedAction::PlacementResizeDown)
        );
        assert_eq!(
            named_action(0, XK_ESCAPE),
            Some(NamedAction::PlacementCancel)
        );
    }
}
