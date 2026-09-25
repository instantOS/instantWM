//! Mouse button bindings.

use super::keybindings::{CONTROL, MOD1, MODKEY, SHIFT};
use crate::actions::{ButtonAction, NamedAction};
use crate::config::commands_common::{ROFI_WINDOW_SWITCH, defaults, media, menu};
use crate::types::{BarPosition, Button, ButtonTarget, ModMask, MouseButton, WindowId};

// `union` rather than `|` because these are `const` items, and `BitOr` is not
// usable in const context on stable.
const MS: ModMask = MODKEY.union(SHIFT);
const MC: ModMask = MODKEY.union(CONTROL);

macro_rules! btn {
    (screen:$target:expr, $mask:expr, button:$btn:expr => $action:expr) => {
        Button {
            target: $target,
            mask: $mask,
            button: $btn,
            action: $action,
        }
    };
    ($target:expr, $mask:expr, button:$btn:expr => $action:expr) => {
        Button {
            target: ButtonTarget::Bar($target),
            mask: $mask,
            button: $btn,
            action: $action,
        }
    };
}

pub fn get_buttons() -> Vec<Button> {
    use BarPosition::*;
    use ButtonTarget::{ClientWin, Root};

    vec![
        btn!(LayoutSymbol, ModMask::NONE, button:MouseButton::Left => ButtonAction::named(NamedAction::CycleLayoutPrev)),
        btn!(LayoutSymbol, ModMask::NONE, button:MouseButton::Right => ButtonAction::named(NamedAction::CycleLayoutNext)),
        btn!(LayoutSymbol, ModMask::NONE, button:MouseButton::Middle => ButtonAction::named(NamedAction::ResetLayout)),
        btn!(LayoutSymbol, MODKEY, button:MouseButton::Left => ButtonAction::named(NamedAction::EdgeScratchpadCreate)),
        btn!(WinTitle(WindowId(0)), ModMask::NONE, button:MouseButton::Left => ButtonAction::WindowTitleMouseHandler),
        btn!(WinTitle(WindowId(0)), ModMask::NONE, button:MouseButton::Middle => ButtonAction::CloseClickedTitleWindow),
        btn!(WinTitle(WindowId(0)), ModMask::NONE, button:MouseButton::Right => ButtonAction::WindowTitleMouseHandler),
        btn!(WinTitle(WindowId(0)), MODKEY, button:MouseButton::Left => ButtonAction::named(NamedAction::EdgeScratchpadToggle)),
        btn!(WinTitle(WindowId(0)), MODKEY, button:MouseButton::Right => ButtonAction::spawn(&["instantnotify"])),
        btn!(WinTitle(WindowId(0)), ModMask::NONE, button:MouseButton::ScrollUp => ButtonAction::named(NamedAction::FocusPrev)),
        btn!(WinTitle(WindowId(0)), ModMask::NONE, button:MouseButton::ScrollDown => ButtonAction::named(NamedAction::FocusNext)),
        btn!(WinTitle(WindowId(0)), SHIFT, button:MouseButton::ScrollUp => ButtonAction::ReorderSelected { direction: crate::types::StackDirection::Previous }),
        btn!(WinTitle(WindowId(0)), SHIFT, button:MouseButton::ScrollDown => ButtonAction::ReorderSelected { direction: crate::types::StackDirection::Next }),
        btn!(WinTitle(WindowId(0)), CONTROL, button:MouseButton::ScrollUp => ButtonAction::ScaleSelected { percent: 110 }),
        btn!(WinTitle(WindowId(0)), CONTROL, button:MouseButton::ScrollDown => ButtonAction::ScaleSelected { percent: 90 }),
        btn!(StatusText, ModMask::NONE, button:MouseButton::Left => ButtonAction::spawn(defaults::APPMENU)),
        btn!(StatusText, ModMask::NONE, button:MouseButton::Middle => ButtonAction::spawn(&["kitty"])),
        btn!(StatusText, ModMask::NONE, button:MouseButton::Right => ButtonAction::spawn(ROFI_WINDOW_SWITCH)),
        btn!(StatusText, ModMask::NONE, button:MouseButton::ScrollUp => ButtonAction::spawn(media::UP_VOL)),
        btn!(StatusText, ModMask::NONE, button:MouseButton::ScrollDown => ButtonAction::spawn(media::DOWN_VOL)),
        btn!(StatusText, MODKEY, button:MouseButton::Left => ButtonAction::spawn(&["ins", "settings", "--gui"])),
        btn!(StatusText, MODKEY, button:MouseButton::Middle => ButtonAction::spawn(media::MUTE_VOL)),
        btn!(StatusText, MODKEY, button:MouseButton::Right => ButtonAction::spawn(&["spoticli", "m"])),
        btn!(StatusText, MODKEY, button:MouseButton::ScrollUp => ButtonAction::spawn(media::UP_BRIGHT)),
        btn!(StatusText, MODKEY, button:MouseButton::ScrollDown => ButtonAction::spawn(media::DOWN_BRIGHT)),
        btn!(StatusText, MS, button:MouseButton::Left => ButtonAction::spawn(&["pavucontrol"])),
        btn!(StatusText, MC, button:MouseButton::Left => ButtonAction::spawn(&["instantnotify"])),
        btn!(Tag(0), ModMask::NONE, button:MouseButton::Left => ButtonAction::DragTagBegin),
        btn!(Tag(0), ModMask::NONE, button:MouseButton::Right => ButtonAction::ToggleClickedViewTag),
        btn!(Tag(0), ModMask::NONE, button:MouseButton::ScrollUp => ButtonAction::named(NamedAction::ScrollLeft)),
        btn!(Tag(0), ModMask::NONE, button:MouseButton::ScrollDown => ButtonAction::named(NamedAction::ScrollRight)),
        btn!(Tag(0), MODKEY, button:MouseButton::Left => ButtonAction::SetSelectedClientClickedTag),
        btn!(Tag(0), MODKEY, button:MouseButton::Right => ButtonAction::ToggleSelectedClientClickedTag),
        btn!(Tag(0), MOD1, button:MouseButton::Left => ButtonAction::DragTagBegin),
        btn!(Tag(0), MODKEY, button:MouseButton::ScrollUp => ButtonAction::named(NamedAction::ShiftViewLeft)),
        btn!(Tag(0), MODKEY, button:MouseButton::ScrollDown => ButtonAction::named(NamedAction::ShiftViewRight)),
        btn!(screen:ButtonTarget::BottomBar, ModMask::NONE, button:MouseButton::Left => ButtonAction::BottomBarDrag {
            left: Box::new(ButtonAction::named(NamedAction::ScrollLeft)),
            right: Box::new(ButtonAction::named(NamedAction::ScrollRight)),
            up: Box::new(ButtonAction::named(NamedAction::ToggleOverview)),
            click: Box::new(ButtonAction::spawn(defaults::APPMENU)),
            hold: Box::new(ButtonAction::spawn(defaults::SETTINGS)),
        }),
        btn!(screen:Root, ModMask::NONE, button:MouseButton::Left => ButtonAction::spawn(defaults::APPMENU)),
        btn!(screen:Root, ModMask::NONE, button:MouseButton::Middle => ButtonAction::spawn(menu::RUN)),
        btn!(screen:Root, ModMask::NONE, button:MouseButton::Right => ButtonAction::spawn(menu::SMART)),
        btn!(screen:Root, ModMask::NONE, button:MouseButton::ScrollUp => ButtonAction::HideEdgeScratchpad),
        btn!(screen:Root, ModMask::NONE, button:MouseButton::ScrollDown => ButtonAction::ShowEdgeScratchpad),
        btn!(screen:Root, MODKEY, button:MouseButton::Left => ButtonAction::named(NamedAction::EdgeScratchpadToggle)),
        btn!(screen:Root, MODKEY, button:MouseButton::Right => ButtonAction::spawn(&["instantnotify"])),
        btn!(screen:ClientWin, MODKEY, button:MouseButton::Left => ButtonAction::ClientMoveDrag),
        btn!(screen:ClientWin, MODKEY, button:MouseButton::Middle => ButtonAction::ToggleFloatingSelected),
        btn!(screen:ClientWin, MODKEY, button:MouseButton::Right => ButtonAction::ResizeMouseFromCursor),
        btn!(screen:ClientWin, MODKEY | MOD1, button:MouseButton::Right => ButtonAction::ResizeMouseFromCursor),
        btn!(screen:ClientWin, MS, button:MouseButton::Right => ButtonAction::ResizeSelectedAspect),
        btn!(CloseButton(WindowId(0)), ModMask::NONE, button:MouseButton::Left => ButtonAction::KillSelectedClient),
        btn!(CloseButton(WindowId(0)), ModMask::NONE, button:MouseButton::Right => ButtonAction::ToggleLockSelectedClient),
        btn!(ResizeWidget(WindowId(0)), ModMask::NONE, button:MouseButton::Left => ButtonAction::named(NamedAction::DrawWindow)),
        btn!(ShutDown, ModMask::NONE, button:MouseButton::Left => ButtonAction::spawn(&["instantshutdown"])),
        btn!(ShutDown, ModMask::NONE, button:MouseButton::Middle => ButtonAction::spawn(&["instantlock", "-o"])),
        btn!(ShutDown, ModMask::NONE, button:MouseButton::Right => ButtonAction::spawn(defaults::LOCKSCREEN)),
        btn!(StartMenu, ModMask::NONE, button:MouseButton::Left => ButtonAction::spawn(&["instantstartmenu"])),
        btn!(StartMenu, ModMask::NONE, button:MouseButton::Right => ButtonAction::spawn(&["quickmenu"])),
        btn!(StartMenu, SHIFT, button:MouseButton::Left => ButtonAction::named(NamedAction::ModeToggle("prefix".into()))),
    ]
}
