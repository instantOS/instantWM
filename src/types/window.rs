//! Window system and resource types.
//!
//! Types for keyboard bindings, mouse buttons, and X commands.

use crate::actions::{ButtonAction, KeyAction};
use crate::types::input::{BarPosition, MouseButton};
use std::fmt::Debug;
use std::sync::Arc;

/// Backend-agnostic window identifier.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct WindowId(pub u32);

impl From<u32> for WindowId {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<WindowId> for u32 {
    fn from(value: WindowId) -> Self {
        value.0
    }
}

impl std::borrow::Borrow<u32> for WindowId {
    fn borrow(&self) -> &u32 {
        &self.0
    }
}

/// Arguments passed to a button action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ButtonArg {
    pub target: ButtonTarget,
    pub window: Option<WindowId>,
    pub btn: MouseButton,
    pub rx: i32,
    pub ry: i32,
}

impl ButtonArg {
    #[inline]
    pub fn bar_position(self) -> Option<BarPosition> {
        match self.target {
            ButtonTarget::Bar(pos) => Some(pos),
            _ => None,
        }
    }
}

/// A keyboard binding.
#[derive(Clone)]
pub struct Key {
    /// Modifier mask (e.g., Mod1Mask, ControlMask).
    pub mod_mask: u32,
    /// Keysym value.
    pub keysym: u32,
    /// Action to execute when key is pressed.
    pub action: KeyAction,
}

impl Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Key")
            .field("mod_mask", &self.mod_mask)
            .field("keysym", &self.keysym)
            .field("action", &self.action)
            .finish()
    }
}

/// Mouse binding target.
///
/// Bar-local hit details stay in [`BarPosition`]; screen-level regions live
/// here so non-bar clicks are not forced through bar terminology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonTarget {
    Bar(BarPosition),
    ClientWin,
    SideBar,
    #[default]
    Root,
}

/// A mouse button binding.
///
/// For bindings that fire on any click of a given kind (e.g. any `Tag`, any
/// `WinTitle`) only the variant discriminant is matched. The inner value is
/// passed to the action at call time so the handler always knows the exact target.
#[derive(Clone)]
pub struct Button {
    /// Which bar/screen region this binding applies to.
    pub target: ButtonTarget,
    /// Modifier mask.
    pub mask: u32,
    /// Mouse button.
    pub button: MouseButton,
    /// Action to execute when button is pressed.
    ///
    /// The exact target is provided to the executor via [`ButtonArg`].
    pub action: ButtonAction,
}

impl Button {
    /// Returns `true` when `target` is the same variant as `self.target`.
    ///
    /// Inner values (tag index, window handle) are intentionally ignored
    /// during matching — they are passed to the action instead.
    pub fn matches(&self, target: ButtonTarget) -> bool {
        match (self.target, target) {
            (ButtonTarget::Bar(binding), ButtonTarget::Bar(actual)) => {
                std::mem::discriminant(&binding) == std::mem::discriminant(&actual)
            }
            (binding, actual) => {
                std::mem::discriminant(&binding) == std::mem::discriminant(&actual)
            }
        }
    }
}

impl Debug for Button {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Button")
            .field("target", &self.target)
            .field("mask", &self.mask)
            .field("button", &self.button)
            .field("action", &self.action)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{Button, ButtonTarget, WindowId};
    use crate::actions::ButtonAction;
    use crate::types::{BarPosition, MouseButton};

    fn button(target: ButtonTarget) -> Button {
        Button {
            target,
            mask: 0,
            button: MouseButton::Left,
            action: ButtonAction::SidebarGestureBegin,
        }
    }

    #[test]
    fn button_target_matches_bar_variant_without_inner_value() {
        let binding = button(ButtonTarget::Bar(BarPosition::WinTitle(WindowId(0))));

        assert!(binding.matches(ButtonTarget::Bar(BarPosition::WinTitle(WindowId(42)))));
        assert!(!binding.matches(ButtonTarget::Bar(BarPosition::Tag(0))));
    }

    #[test]
    fn button_target_keeps_non_bar_targets_separate() {
        let sidebar = button(ButtonTarget::SideBar);

        assert!(sidebar.matches(ButtonTarget::SideBar));
        assert!(!sidebar.matches(ButtonTarget::Root));
        assert!(!sidebar.matches(ButtonTarget::Bar(BarPosition::Root)));
    }
}

/// System tray state.
#[derive(Debug, Clone)]
pub struct Systray {
    /// Tray window handle.
    pub win: WindowId,
    /// List of tray icon windows.
    pub icons: Vec<WindowId>,
}

/// Wayland StatusNotifier tray icon model.
#[derive(Debug, Clone, Default)]
pub struct WaylandSystrayItem {
    pub service: String,
    pub path: String,
    pub icon_rgba: Arc<[u8]>,
    pub icon_w: i32,
    pub icon_h: i32,
}

/// Wayland StatusNotifier tray state.
#[derive(Debug, Clone, Default)]
pub struct WaylandSystray {
    pub items: Vec<WaylandSystrayItem>,
}

#[derive(Debug, Clone, Default)]
pub struct WaylandSystrayMenuItem {
    pub id: i32,
    pub label: String,
    pub width: i32,
    pub enabled: bool,
    pub separator: bool,
}

#[derive(Debug, Clone, Default)]
pub struct WaylandSystrayMenu {
    pub service: String,
    pub path: String,
    pub item_h: i32,
    pub items: Vec<WaylandSystrayMenuItem>,
}
