//! Keyboard bindings: normal keys (`default_keybinds`) and prefix-mode keys (`get_desktop_keybinds`).

use crate::actions::{KeyAction, NamedAction};
use crate::backend::BackendKind;
use crate::config::commands_common::{ROFI_WINDOW_SWITCH, defaults, media, menu, screenshot};
use crate::config::generated_keybinds::{
    backend_launcher, resolve_lockscreen_command, resolve_terminal_command,
};
use crate::layouts::LayoutCommand;
use crate::types::{Key, KeybindOrigin, ModMask, Modifier, MonitorDirection};

use super::keysyms::*;

// instantWM's primary modifier is pinned to the X11 `Mod4Mask` position. This is
// the WM's own choice rather than something the protocol dictates, and it is the
// reason [`Modifier`] names bit 6 `Super` while calling bits 4, 5 and 7 `Mod2`,
// `Mod3` and `Mod5` — those have no universal meaning to name semantically.
pub const MODKEY: ModMask = ModMask::from_modifier(Modifier::Super);
pub const CONTROL: ModMask = ModMask::from_modifier(Modifier::Control);
pub const SHIFT: ModMask = ModMask::from_modifier(Modifier::Shift);
pub const MOD1: ModMask = ModMask::from_modifier(Modifier::Alt);
/// X-protocol modifier bit for Mod2 (usually Num Lock).
pub const MOD2: ModMask = ModMask::from_modifier(Modifier::Mod2);
/// X-protocol modifier bit for Mod3.
pub const MOD3: ModMask = ModMask::from_modifier(Modifier::Mod3);
/// X-protocol modifier bit for Mod5 (usually AltGr).
pub const MOD5: ModMask = ModMask::from_modifier(Modifier::Mod5);

macro_rules! key {
    ($mods:expr, $sym:expr => $action:expr) => {
        Key {
            mod_mask: $mods,
            keysym: $sym,
            action: $action,
            origin: KeybindOrigin::CompiledDefault,
        }
    };
}

fn tag_keys(keysym: Keysym, tag_idx: usize) -> [Key; 6] {
    [
        key!(MODKEY, keysym => KeyAction::ViewTag { tag_idx }),
        key!(MODKEY | CONTROL, keysym => KeyAction::ToggleViewTag { tag_idx }),
        key!(MODKEY | SHIFT, keysym => KeyAction::SetClientTag { tag_idx }),
        key!(MODKEY | MOD1, keysym => KeyAction::FollowClientTag { tag_idx }),
        key!(MODKEY | CONTROL | SHIFT, keysym => KeyAction::ToggleClientTag { tag_idx }),
        key!(MODKEY | MOD1 | SHIFT, keysym => KeyAction::SwapTags { tag_idx }),
    ]
}

/// The compiled default key table for `backend`.
pub fn default_keybinds(backend: BackendKind) -> Vec<Key> {
    let mut keys: Vec<Key> = vec![
        key!(MODKEY | MOD1, XK_J => KeyAction::named(NamedAction::KeyResizeDown)),
        key!(MODKEY | MOD1, XK_K => KeyAction::named(NamedAction::KeyResizeUp)),
        key!(MODKEY | MOD1, XK_L => KeyAction::named(NamedAction::KeyResizeRight)),
        key!(MODKEY | MOD1, XK_H => KeyAction::named(NamedAction::KeyResizeLeft)),
        // Super+plus/minus grow and shrink the focused window along its most
        // local split. The '+' chord covers every keymap spelling: unshifted
        // '+' keys, shift held on such keys, and US layouts where typed '+'
        // resolves from the '=' base keysym.
        key!(MODKEY, XK_PLUS => KeyAction::named(NamedAction::TreeGrow)),
        key!(MODKEY | SHIFT, XK_PLUS => KeyAction::named(NamedAction::TreeGrow)),
        key!(MODKEY | SHIFT, XK_EQUAL => KeyAction::named(NamedAction::TreeGrow)),
        key!(MODKEY, XK_MINUS => KeyAction::named(NamedAction::TreeShrink)),
        // Super+Ctrl+plus/minus resize the tiling gaps. Control never changes
        // the base keysym, so these chords mean the same thing on every keymap;
        // the shifted spellings cover layouts that type '+' or '-' with shift.
        key!(MODKEY | CONTROL, XK_PLUS => KeyAction::named(NamedAction::IncGaps(None))),
        key!(MODKEY | CONTROL, XK_EQUAL => KeyAction::named(NamedAction::IncGaps(None))),
        key!(MODKEY | CONTROL | SHIFT, XK_PLUS => KeyAction::named(NamedAction::IncGaps(None))),
        key!(MODKEY | CONTROL | SHIFT, XK_EQUAL => KeyAction::named(NamedAction::IncGaps(None))),
        key!(MODKEY | CONTROL, XK_MINUS => KeyAction::named(NamedAction::DecGaps(None))),
        key!(MODKEY | CONTROL | SHIFT, XK_MINUS => KeyAction::named(NamedAction::DecGaps(None))),
        key!(MODKEY | SHIFT, XK_J => KeyAction::named(NamedAction::KeyMoveDown)),
        key!(MODKEY | SHIFT, XK_K => KeyAction::named(NamedAction::KeyMoveUp)),
        key!(MODKEY | SHIFT, XK_L => KeyAction::named(NamedAction::KeyMoveRight)),
        key!(MODKEY | SHIFT, XK_H => KeyAction::named(NamedAction::KeyMoveLeft)),
        key!(MODKEY, XK_I => KeyAction::named(NamedAction::IncMasterCount(Some(1)))),
        key!(MODKEY, XK_D => KeyAction::named(NamedAction::IncMasterCount(Some(-1)))),
        key!(MODKEY, XK_H => KeyAction::named(NamedAction::FocusLeft)),
        key!(MODKEY, XK_J => KeyAction::named(NamedAction::FocusDown)),
        key!(MODKEY, XK_K => KeyAction::named(NamedAction::FocusUp)),
        key!(MODKEY, XK_L => KeyAction::named(NamedAction::FocusRight)),
        key!(MODKEY, XK_T => KeyAction::named(NamedAction::EdgeScratchpadToggle)),
        key!(MODKEY, XK_C => KeyAction::named(NamedAction::SetLayout(LayoutCommand::Grid))),
        key!(MODKEY, XK_F => KeyAction::named(NamedAction::LayoutFloat)),
        key!(MODKEY, XK_W => KeyAction::named(NamedAction::ToggleTilingMaximized)),
        key!(MODKEY | CONTROL, XK_COMMA => KeyAction::named(NamedAction::CycleLayoutPrev)),
        key!(MODKEY | CONTROL, XK_PERIOD => KeyAction::named(NamedAction::CycleLayoutNext)),
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
        key!(MODKEY, XK_COMMA => KeyAction::named(NamedAction::FocusMon(MonitorDirection::Prev))),
        key!(MODKEY, XK_PERIOD => KeyAction::named(NamedAction::FocusMon(MonitorDirection::Next))),
        key!(MODKEY | MOD1, XK_COMMA => KeyAction::named(NamedAction::FollowMon(MonitorDirection::Prev))),
        key!(MODKEY | MOD1, XK_PERIOD => KeyAction::named(NamedAction::FollowMon(MonitorDirection::Next))),
        // Super+Shift+,/. move the focused client without following it. Shift is
        // part of the modifier mask; the base comma/period keysym is unchanged.
        key!(MODKEY | SHIFT, XK_COMMA => KeyAction::named(NamedAction::SendMon(MonitorDirection::Prev))),
        key!(MODKEY | SHIFT, XK_PERIOD => KeyAction::named(NamedAction::SendMon(MonitorDirection::Next))),
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
        key!(MODKEY | SHIFT, XK_S => KeyAction::named(NamedAction::ScratchpadRestore)),
        key!(MODKEY, XK_B => KeyAction::named(NamedAction::ToggleBar)),
        key!(MODKEY | SHIFT, XK_B => KeyAction::named(NamedAction::ToggleBottomBar(None))),
        key!(MODKEY | CONTROL, XK_S => KeyAction::named(NamedAction::ToggleSticky)),
        key!(MODKEY | MOD1, XK_S => KeyAction::named(NamedAction::ConfigToggle("tags.show_icons".into()))),
        key!(MODKEY | SHIFT | MOD1, XK_S => KeyAction::named(NamedAction::ConfigToggle("animations.enabled".into()))),
        key!(MODKEY | MOD1, XK_SPACE => KeyAction::named(NamedAction::NextKeyboardLayout)),
        key!(MODKEY | SHIFT | CONTROL | MOD1, XK_TAB => KeyAction::named(NamedAction::ModeToggle("desktop".into()))),
        key!(MODKEY | CONTROL, XK_H => KeyAction::named(NamedAction::Hide)),
        key!(MODKEY | CONTROL | MOD1, XK_H => KeyAction::named(NamedAction::UnhideAll)),
        key!(MODKEY, XK_Q => KeyAction::named(NamedAction::ShutKill)),
        key!(MOD1, XK_F4 => KeyAction::named(NamedAction::Kill)),
        key!(MODKEY | SHIFT | CONTROL, XK_Q => KeyAction::named(NamedAction::Quit)),
        key!(MODKEY, XK_F2 => KeyAction::named(NamedAction::ModeToggle("prefix".into()))),
        key!(MODKEY, XK_RETURN => KeyAction::spawn(&[resolve_terminal_command()])),
        key!(MODKEY, XK_SPACE => KeyAction::spawn(&[backend_launcher(backend)])),
        key!(MODKEY | CONTROL, XK_SPACE => KeyAction::spawn(menu::RUN)),
        key!(MODKEY, XK_V => KeyAction::spawn(menu::CLIP)),
        key!(MODKEY | MOD1, XK_MINUS => KeyAction::spawn(menu::ST)),
        key!(MODKEY | SHIFT, XK_V => KeyAction::spawn(menu::QUICK)),
        key!(MODKEY, XK_N => KeyAction::spawn(defaults::FILEMANAGER)),
        key!(MODKEY, XK_R => KeyAction::spawn(defaults::TERM_FILEMANAGER)),
        key!(MODKEY, XK_Y => KeyAction::spawn(defaults::APPMENU)),
        key!(MODKEY, XK_X => KeyAction::spawn(&["iswitch"])),
        key!(MODKEY, XK_A => KeyAction::spawn(&["ins", "assist"])),
        key!(MODKEY, XK_QUESTION => KeyAction::spawn(defaults::KEYHELP)),
        key!(MODKEY | SHIFT, XK_QUESTION => KeyAction::spawn(defaults::KEYHELP)),
        key!(MODKEY | SHIFT, XK_SLASH => KeyAction::spawn(defaults::KEYHELP)),
        key!(MOD1, XK_TAB => KeyAction::spawn(&["iswitch"])),
        key!(MODKEY, XK_DEAD_CIRCUMFLEX => KeyAction::spawn(ROFI_WINDOW_SWITCH)),
        key!(MODKEY | CONTROL, XK_L => KeyAction::spawn(&[resolve_lockscreen_command(backend)])),
        key!(MODKEY | CONTROL, XK_C => KeyAction::spawn(defaults::SETTINGS)),
        key!(MODKEY | CONTROL, XK_Q => KeyAction::spawn(&["instantshutdown"])),
        key!(MODKEY | MOD1, XK_F => KeyAction::spawn(&["instantsearch"])),
        key!(MODKEY | SHIFT, XK_ESCAPE => KeyAction::spawn(defaults::SYSTEMMONITOR)),
        key!(MODKEY, XK_PRINT => KeyAction::spawn(screenshot::AREA)),
        key!(MODKEY | SHIFT, XK_PRINT => KeyAction::spawn(screenshot::FULL)),
        key!(MODKEY | CONTROL, XK_PRINT => KeyAction::spawn(screenshot::CLIPBOARD)),
        key!(MODKEY | MOD1, XK_PRINT => KeyAction::spawn(screenshot::FULL_CLIPBOARD)),
        key!(ModMask::NONE, XF86XK_MON_BRIGHTNESS_UP => KeyAction::spawn(media::UP_BRIGHT)),
        key!(ModMask::NONE, XF86XK_MON_BRIGHTNESS_DOWN => KeyAction::spawn(media::DOWN_BRIGHT)),
        key!(ModMask::NONE, XF86XK_AUDIO_LOWER_VOLUME => KeyAction::spawn(media::DOWN_VOL)),
        key!(ModMask::NONE, XF86XK_AUDIO_MUTE => KeyAction::spawn(media::MUTE_VOL)),
        key!(ModMask::NONE, XF86XK_AUDIO_RAISE_VOLUME => KeyAction::spawn(media::UP_VOL)),
        key!(ModMask::NONE, XF86XK_AUDIO_MIC_MUTE => KeyAction::spawn(media::MIC_MUTE)),
        key!(ModMask::NONE, XF86XK_AUDIO_PLAY => KeyAction::spawn(&["playerctl", "play-pause"])),
        key!(ModMask::NONE, XF86XK_AUDIO_PAUSE => KeyAction::spawn(&["playerctl", "play-pause"])),
        key!(ModMask::NONE, XF86XK_AUDIO_NEXT => KeyAction::spawn(&["playerctl", "next"])),
        key!(ModMask::NONE, XF86XK_AUDIO_PREV => KeyAction::spawn(&["playerctl", "previous"])),
    ];

    for tag_idx in 0..9 {
        keys.extend_from_slice(&tag_keys(Keysym::new(XK_1.raw() + tag_idx as u32), tag_idx));
    }

    keys
}

pub fn get_desktop_keybinds() -> Vec<Key> {
    vec![
        key!(ModMask::NONE, XK_RETURN => KeyAction::spawn(defaults::TERMINAL)),
        key!(ModMask::NONE, XK_R => KeyAction::spawn(defaults::TERM_FILEMANAGER)),
        key!(ModMask::NONE, XK_E => KeyAction::spawn(defaults::EDITOR)),
        key!(ModMask::NONE, XK_N => KeyAction::spawn(defaults::FILEMANAGER)),
        key!(ModMask::NONE, XK_SPACE => KeyAction::spawn(defaults::APPMENU)),
        key!(ModMask::NONE, XK_Y => KeyAction::spawn(menu::SMART)),
        key!(ModMask::NONE, XK_F => KeyAction::spawn(defaults::BROWSER)),
        key!(ModMask::NONE, XK_TAB => KeyAction::spawn(ROFI_WINDOW_SWITCH)),
        key!(ModMask::NONE, XK_PLUS => KeyAction::spawn(media::UP_VOL)),
        key!(ModMask::NONE, XK_MINUS => KeyAction::spawn(media::DOWN_VOL)),
        key!(ModMask::NONE, XK_H => KeyAction::named(NamedAction::ScrollLeft)),
        key!(ModMask::NONE, XK_L => KeyAction::named(NamedAction::ScrollRight)),
        key!(ModMask::NONE, XK_LEFT => KeyAction::named(NamedAction::ScrollLeft)),
        key!(ModMask::NONE, XK_RIGHT => KeyAction::named(NamedAction::ScrollRight)),
        key!(ModMask::NONE, XK_K => KeyAction::named(NamedAction::ShiftViewRight)),
        key!(ModMask::NONE, XK_J => KeyAction::named(NamedAction::ShiftViewLeft)),
        key!(ModMask::NONE, XK_UP => KeyAction::named(NamedAction::ShiftViewRight)),
        key!(ModMask::NONE, XK_DOWN => KeyAction::named(NamedAction::ShiftViewLeft)),
        key!(ModMask::NONE, XK_1 => KeyAction::ViewTag { tag_idx: 0 }),
        key!(ModMask::NONE, XK_2 => KeyAction::ViewTag { tag_idx: 1 }),
        key!(ModMask::NONE, XK_3 => KeyAction::ViewTag { tag_idx: 2 }),
        key!(ModMask::NONE, XK_4 => KeyAction::ViewTag { tag_idx: 3 }),
        key!(ModMask::NONE, XK_5 => KeyAction::ViewTag { tag_idx: 4 }),
        key!(ModMask::NONE, XK_6 => KeyAction::ViewTag { tag_idx: 5 }),
        key!(ModMask::NONE, XK_7 => KeyAction::ViewTag { tag_idx: 6 }),
        key!(ModMask::NONE, XK_8 => KeyAction::ViewTag { tag_idx: 7 }),
        key!(ModMask::NONE, XK_9 => KeyAction::ViewTag { tag_idx: 8 }),
    ]
}

/// Default bindings for the compositor-owned tree placement mode. Super is
/// intentionally absent: the dispatcher ignores a still-held Super modifier
/// after the Super+M entry chord, while every binding remains configurable as
/// an ordinary named action under `[modes.placement]`.
pub fn get_tree_placement_keybinds() -> Vec<Key> {
    vec![
        key!(ModMask::NONE, XK_LEFT => KeyAction::named(NamedAction::PlacementLeft)),
        key!(ModMask::NONE, XK_H => KeyAction::named(NamedAction::PlacementLeft)),
        key!(ModMask::NONE, XK_RIGHT => KeyAction::named(NamedAction::PlacementRight)),
        key!(ModMask::NONE, XK_L => KeyAction::named(NamedAction::PlacementRight)),
        key!(ModMask::NONE, XK_UP => KeyAction::named(NamedAction::PlacementUp)),
        key!(ModMask::NONE, XK_K => KeyAction::named(NamedAction::PlacementUp)),
        key!(ModMask::NONE, XK_DOWN => KeyAction::named(NamedAction::PlacementDown)),
        key!(ModMask::NONE, XK_J => KeyAction::named(NamedAction::PlacementDown)),
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
        key!(ModMask::NONE, XK_TAB => KeyAction::named(NamedAction::PlacementNext)),
        key!(SHIFT, XK_TAB => KeyAction::named(NamedAction::PlacementPrevious)),
        key!(ModMask::NONE, XK_SPACE => KeyAction::named(NamedAction::PlacementCenter)),
        key!(ModMask::NONE, XK_RETURN => KeyAction::named(NamedAction::PlacementApply)),
        key!(ModMask::NONE, XK_ESCAPE => KeyAction::named(NamedAction::PlacementCancel)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named_action(modifiers: ModMask, keysym: Keysym) -> Option<NamedAction> {
        get_tree_placement_keybinds()
            .into_iter()
            .find(|key| key.mod_mask == modifiers && key.keysym == keysym)
            .and_then(|key| match key.action {
                KeyAction::Named(action) => Some(action),
                _ => None,
            })
    }

    fn default_named_action(modifiers: ModMask, keysym: Keysym) -> Option<NamedAction> {
        default_keybinds(BackendKind::Wayland)
            .into_iter()
            .find(|key| key.mod_mask == modifiers && key.keysym == keysym)
            .and_then(|key| match key.action {
                KeyAction::Named(action) => Some(action),
                _ => None,
            })
    }

    #[test]
    fn scratchpad_restore_has_a_default_binding() {
        assert_eq!(
            default_named_action(MODKEY | SHIFT, XK_S),
            Some(NamedAction::ScratchpadRestore)
        );
    }

    #[test]
    fn gap_defaults_are_on_super_ctrl_plus_and_minus() {
        // Plain Super+plus/minus keep growing and shrinking the focused
        // window's split, in every base-keysym spelling.
        assert_eq!(
            default_named_action(MODKEY, XK_PLUS),
            Some(NamedAction::TreeGrow)
        );
        assert_eq!(
            default_named_action(MODKEY | SHIFT, XK_PLUS),
            Some(NamedAction::TreeGrow)
        );
        assert_eq!(
            default_named_action(MODKEY | SHIFT, XK_EQUAL),
            Some(NamedAction::TreeGrow)
        );
        assert_eq!(
            default_named_action(MODKEY, XK_MINUS),
            Some(NamedAction::TreeShrink)
        );

        // The Super+Ctrl chords resize gaps and are keymap independent; the
        // shifted spellings cover layouts that type '+' or '-' with shift.
        assert_eq!(
            default_named_action(MODKEY | CONTROL, XK_PLUS),
            Some(NamedAction::IncGaps(None))
        );
        assert_eq!(
            default_named_action(MODKEY | CONTROL, XK_EQUAL),
            Some(NamedAction::IncGaps(None))
        );
        assert_eq!(
            default_named_action(MODKEY | CONTROL | SHIFT, XK_EQUAL),
            Some(NamedAction::IncGaps(None))
        );
        assert_eq!(
            default_named_action(MODKEY | CONTROL | SHIFT, XK_PLUS),
            Some(NamedAction::IncGaps(None))
        );
        assert_eq!(
            default_named_action(MODKEY | CONTROL, XK_MINUS),
            Some(NamedAction::DecGaps(None))
        );
        assert_eq!(
            default_named_action(MODKEY | CONTROL | SHIFT, XK_MINUS),
            Some(NamedAction::DecGaps(None))
        );
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
    fn vim_and_arrow_focus_bindings_are_equivalent() {
        for (vim, arrow, action) in [
            (XK_H, XK_LEFT, NamedAction::FocusLeft),
            (XK_J, XK_DOWN, NamedAction::FocusDown),
            (XK_K, XK_UP, NamedAction::FocusUp),
            (XK_L, XK_RIGHT, NamedAction::FocusRight),
        ] {
            assert_eq!(default_named_action(MODKEY, vim), Some(action.clone()));
            assert_eq!(default_named_action(MODKEY, arrow), Some(action));
        }
    }

    #[test]
    fn monitor_transfer_defaults_separate_following_from_plain_move() {
        // Super+Alt+,/. carry the focused client to the adjacent monitor and
        // follow it there.
        assert_eq!(
            default_named_action(MODKEY | MOD1, XK_COMMA),
            Some(NamedAction::FollowMon(MonitorDirection::Prev))
        );
        assert_eq!(
            default_named_action(MODKEY | MOD1, XK_PERIOD),
            Some(NamedAction::FollowMon(MonitorDirection::Next))
        );

        // Super+Shift+,/. move the focused client without following it, so the
        // plain comma/period focus bindings stay untouched.
        assert_eq!(
            default_named_action(MODKEY | SHIFT, XK_COMMA),
            Some(NamedAction::SendMon(MonitorDirection::Prev))
        );
        assert_eq!(
            default_named_action(MODKEY | SHIFT, XK_PERIOD),
            Some(NamedAction::SendMon(MonitorDirection::Next))
        );

        assert_eq!(
            default_named_action(MODKEY, XK_COMMA),
            Some(NamedAction::FocusMon(MonitorDirection::Prev))
        );
        assert_eq!(
            default_named_action(MODKEY, XK_PERIOD),
            Some(NamedAction::FocusMon(MonitorDirection::Next))
        );
    }

    #[test]
    fn placement_defaults_are_regular_named_actions() {
        assert_eq!(
            named_action(ModMask::NONE, XK_H),
            Some(NamedAction::PlacementLeft)
        );
        assert_eq!(
            named_action(SHIFT, XK_LEFT),
            Some(NamedAction::PlacementSwapLeft)
        );
        assert_eq!(
            named_action(CONTROL, XK_J),
            Some(NamedAction::PlacementResizeDown)
        );
        assert_eq!(
            named_action(ModMask::NONE, XK_ESCAPE),
            Some(NamedAction::PlacementCancel)
        );
    }

    #[test]
    fn system_dependent_spawn_defaults_are_resolved_into_the_table() {
        assert_eq!(
            default_spawn_args_for(BackendKind::X11, MODKEY, XK_SPACE),
            Some(vec!["instantmenu_smartrun".to_string()])
        );
        assert_eq!(
            default_spawn_args_for(BackendKind::Wayland, MODKEY, XK_SPACE),
            Some(vec!["fuzzel".to_string()])
        );
        for backend in [BackendKind::X11, BackendKind::Wayland] {
            assert!(default_spawn_args_for(backend, MODKEY | CONTROL, XK_L).is_some());
            assert!(default_spawn_args_for(backend, MODKEY, XK_RETURN).is_some());
            let chords = default_keybinds(backend)
                .iter()
                .filter(|key| key.mod_mask == MODKEY && key.keysym == XK_RETURN)
                .count();
            assert_eq!(chords, 1);
        }
    }

    fn default_spawn_args_for(
        backend: BackendKind,
        modifiers: ModMask,
        keysym: Keysym,
    ) -> Option<Vec<String>> {
        default_keybinds(backend)
            .into_iter()
            .find(|key| key.mod_mask == modifiers && key.keysym == keysym)
            .and_then(|key| match key.action {
                KeyAction::Named(NamedAction::Spawn(args)) => Some(args),
                _ => None,
            })
    }

    #[test]
    fn super_ctrl_c_launches_settings_gui() {
        let spawn_args = default_keybinds(BackendKind::Wayland)
            .into_iter()
            .find(|key| key.mod_mask == MODKEY | CONTROL && key.keysym == XK_C)
            .and_then(|key| match key.action {
                KeyAction::Named(NamedAction::Spawn(args)) => Some(args),
                _ => None,
            });

        assert_eq!(
            spawn_args,
            Some(vec![
                "ins".to_string(),
                "settings".to_string(),
                "--gui".to_string()
            ])
        );
    }

    fn default_spawn_args(modifiers: ModMask, keysym: Keysym) -> Option<Vec<String>> {
        default_keybinds(BackendKind::Wayland)
            .into_iter()
            .find(|key| key.mod_mask == modifiers && key.keysym == keysym)
            .and_then(|key| match key.action {
                KeyAction::Named(NamedAction::Spawn(args)) => Some(args),
                _ => None,
            })
    }

    #[test]
    fn super_v_launches_clipboard_gui_and_shift_keeps_quickmenu() {
        assert_eq!(
            default_spawn_args(MODKEY, XK_V),
            Some(vec![
                "ins".to_string(),
                "clip".to_string(),
                "--gui".to_string()
            ])
        );
        assert_eq!(
            default_spawn_args(MODKEY | SHIFT, XK_V),
            Some(vec!["quickmenu".to_string()])
        );
    }

    #[test]
    fn mic_mute_key_defaults_to_assist_chord() {
        assert_eq!(
            default_spawn_args(ModMask::NONE, XF86XK_AUDIO_MIC_MUTE),
            Some(vec![
                "ins".to_string(),
                "assist".to_string(),
                "run".to_string(),
                "vm".to_string()
            ])
        );
    }

    #[test]
    fn super_r_launches_terminal_file_manager_through_default_aliases() {
        let spawn_args = default_keybinds(BackendKind::Wayland)
            .into_iter()
            .find(|key| key.mod_mask == MODKEY && key.keysym == XK_R)
            .and_then(|key| match key.action {
                KeyAction::Named(NamedAction::Spawn(args)) => Some(args),
                _ => None,
            });

        assert_eq!(
            spawn_args,
            Some(vec![
                ".config/instantos/default/terminal".to_string(),
                "-e".to_string(),
                ".config/instantos/default/termfilemanager".to_string(),
            ])
        );
    }
}
