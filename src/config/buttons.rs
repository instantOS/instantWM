//! Mouse button bindings.

use super::keybindings::{CONTROL, MOD1, MODKEY, SHIFT};
use crate::actions::{argv, ButtonAction, NamedAction};
use crate::config::commands_common::{ROFI_WINDOW_SWITCH, defaults, media, menu};
use crate::types::{BarPosition, Button, MouseButton, WindowId};

const MS: u32 = MODKEY | SHIFT;
const MC: u32 = MODKEY | CONTROL;

macro_rules! btn {
    ($target:expr, $mask:expr, button:$btn:expr => $action:expr) => {
        Button {
            target: $target,
            mask: $mask,
            button: $btn,
            action: $action,
        }
    };
}

fn named(action: NamedAction) -> ButtonAction {
    ButtonAction::Named {
        action,
        args: Vec::new(),
    }
}

fn named_args(action: NamedAction, args: &[&str]) -> ButtonAction {
    ButtonAction::Named {
        action,
        args: argv(args),
    }
}

pub fn get_buttons() -> Vec<Button> {
    use BarPosition::*;

    vec![
        btn!(LtSymbol, 0, button:MouseButton::Left => named(NamedAction::CycleLayoutPrev)),
        btn!(LtSymbol, 0, button:MouseButton::Right => named(NamedAction::CycleLayoutNext)),
        btn!(LtSymbol, 0, button:MouseButton::Middle => named(NamedAction::LayoutTile)),
        btn!(LtSymbol, MODKEY, button:MouseButton::Left => named(NamedAction::CreateOverlay)),
        btn!(WinTitle(WindowId(0)), 0, button:MouseButton::Left => ButtonAction::WindowTitleMouseHandler),
        btn!(WinTitle(WindowId(0)), 0, button:MouseButton::Middle => ButtonAction::CloseClickedTitleWindow),
        btn!(WinTitle(WindowId(0)), 0, button:MouseButton::Right => ButtonAction::WindowTitleMouseHandler),
        btn!(WinTitle(WindowId(0)), MODKEY, button:MouseButton::Left => named(NamedAction::SetOverlay)),
        btn!(WinTitle(WindowId(0)), MODKEY, button:MouseButton::Right => named_args(NamedAction::Spawn, &["instantnotify"])),
        btn!(WinTitle(WindowId(0)), 0, button:MouseButton::ScrollUp => named(NamedAction::FocusPrev)),
        btn!(WinTitle(WindowId(0)), 0, button:MouseButton::ScrollDown => named(NamedAction::FocusNext)),
        btn!(WinTitle(WindowId(0)), SHIFT, button:MouseButton::ScrollUp => ButtonAction::ReorderSelected { up: true }),
        btn!(WinTitle(WindowId(0)), SHIFT, button:MouseButton::ScrollDown => ButtonAction::ReorderSelected { up: false }),
        btn!(WinTitle(WindowId(0)), CONTROL, button:MouseButton::ScrollUp => ButtonAction::ScaleSelected { percent: 110 }),
        btn!(WinTitle(WindowId(0)), CONTROL, button:MouseButton::ScrollDown => ButtonAction::ScaleSelected { percent: 90 }),
        btn!(StatusText, 0, button:MouseButton::Left => named_args(NamedAction::Spawn, defaults::APPMENU)),
        btn!(StatusText, 0, button:MouseButton::Middle => named_args(NamedAction::Spawn, &["kitty"])),
        btn!(StatusText, 0, button:MouseButton::Right => named_args(NamedAction::Spawn, ROFI_WINDOW_SWITCH)),
        btn!(StatusText, 0, button:MouseButton::ScrollUp => named_args(NamedAction::Spawn, media::up_vol())),
        btn!(StatusText, 0, button:MouseButton::ScrollDown => named_args(NamedAction::Spawn, media::down_vol())),
        btn!(StatusText, MODKEY, button:MouseButton::Left => named_args(NamedAction::Spawn, &["ins", "settings", "--gui"])),
        btn!(StatusText, MODKEY, button:MouseButton::Middle => named_args(NamedAction::Spawn, media::mute_vol())),
        btn!(StatusText, MODKEY, button:MouseButton::Right => named_args(NamedAction::Spawn, &["spoticli", "m"])),
        btn!(StatusText, MODKEY, button:MouseButton::ScrollUp => named_args(NamedAction::Spawn, media::up_bright())),
        btn!(StatusText, MODKEY, button:MouseButton::ScrollDown => named_args(NamedAction::Spawn, media::down_bright())),
        btn!(StatusText, MS, button:MouseButton::Left => named_args(NamedAction::Spawn, &["pavucontrol"])),
        btn!(StatusText, MC, button:MouseButton::Left => named_args(NamedAction::Spawn, &["instantnotify"])),
        btn!(Tag(0), 0, button:MouseButton::Left => ButtonAction::DragTagBegin),
        btn!(Tag(0), 0, button:MouseButton::Right => ButtonAction::ToggleClickedViewTag),
        btn!(Tag(0), 0, button:MouseButton::ScrollUp => named(NamedAction::ScrollLeft)),
        btn!(Tag(0), 0, button:MouseButton::ScrollDown => named(NamedAction::ScrollRight)),
        btn!(Tag(0), MODKEY, button:MouseButton::Left => ButtonAction::SetSelectedClientClickedTag),
        btn!(Tag(0), MODKEY, button:MouseButton::Right => ButtonAction::ToggleSelectedClientClickedTag),
        btn!(Tag(0), MOD1, button:MouseButton::Left => ButtonAction::FollowSelectedClientClickedTag),
        btn!(Tag(0), MODKEY, button:MouseButton::ScrollUp => named(NamedAction::ShiftViewLeft)),
        btn!(Tag(0), MODKEY, button:MouseButton::ScrollDown => named(NamedAction::ShiftViewRight)),
        btn!(Root, 0, button:MouseButton::Left => named_args(NamedAction::Spawn, defaults::APPMENU)),
        btn!(Root, 0, button:MouseButton::Middle => named_args(NamedAction::Spawn, menu::RUN)),
        btn!(Root, 0, button:MouseButton::Right => named_args(NamedAction::Spawn, menu::SMART)),
        btn!(Root, 0, button:MouseButton::ScrollUp => ButtonAction::HideOverlay),
        btn!(Root, 0, button:MouseButton::ScrollDown => ButtonAction::ShowOverlay),
        btn!(Root, MODKEY, button:MouseButton::Left => named(NamedAction::SetOverlay)),
        btn!(Root, MODKEY, button:MouseButton::Right => named_args(NamedAction::Spawn, &["instantnotify"])),
        btn!(ClientWin, MODKEY, button:MouseButton::Left => ButtonAction::ClientMoveDrag),
        btn!(ClientWin, MODKEY, button:MouseButton::Middle => ButtonAction::ToggleFloatingSelected),
        btn!(ClientWin, MODKEY, button:MouseButton::Right => ButtonAction::ResizeMouseFromCursor),
        btn!(ClientWin, MODKEY | MOD1, button:MouseButton::Right => ButtonAction::ResizeMouseFromCursor),
        btn!(ClientWin, MS, button:MouseButton::Right => ButtonAction::ResizeSelectedAspect),
        btn!(CloseButton(WindowId(0)), 0, button:MouseButton::Left => ButtonAction::KillSelectedClient),
        btn!(CloseButton(WindowId(0)), 0, button:MouseButton::Right => ButtonAction::ToggleLockSelectedClient),
        btn!(ResizeWidget(WindowId(0)), 0, button:MouseButton::Left => named(NamedAction::DrawWindow)),
        btn!(ShutDown, 0, button:MouseButton::Left => named_args(NamedAction::Spawn, &["instantshutdown"])),
        btn!(ShutDown, 0, button:MouseButton::Middle => named_args(NamedAction::Spawn, &["instantlock", "-o"])),
        btn!(ShutDown, 0, button:MouseButton::Right => named_args(NamedAction::Spawn, &[".config/instantos/default/lockscreen"])),
        btn!(SideBar, 0, button:MouseButton::Left => ButtonAction::GestureMouse),
        btn!(StartMenu, 0, button:MouseButton::Left => named_args(NamedAction::Spawn, &["instantstartmenu"])),
        btn!(StartMenu, 0, button:MouseButton::Right => named_args(NamedAction::Spawn, &["quickmenu"])),
        btn!(StartMenu, SHIFT, button:MouseButton::Left => named(NamedAction::TogglePrefix)),
    ]
}
