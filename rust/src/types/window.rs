//! Window system and resource types.
//!
//! Types for keyboard bindings, mouse buttons, and X commands.

use std::fmt::Debug;
use std::rc::Rc;

use crate::contexts::CoreCtx;
use crate::types::input::BarPosition;
use crate::types::input::MouseButton;

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
    pub pos: BarPosition,
    pub btn: MouseButton,
    pub rx: i32,
    pub ry: i32,
}

/// A keyboard binding.
pub struct Key {
    /// Modifier mask (e.g., Mod1Mask, ControlMask).
    pub mod_mask: u32,
    /// Keysym value.
    pub keysym: u32,
    /// Action to execute when key is pressed.
    pub action: Rc<dyn Fn(&mut CoreCtx)>,
}

impl Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Key")
            .field("mod_mask", &self.mod_mask)
            .field("keysym", &self.keysym)
            .field("action", &"<closure>")
            .finish()
    }
}

impl Clone for Key {
    fn clone(&self) -> Self {
        Self {
            mod_mask: self.mod_mask,
            keysym: self.keysym,
            action: Rc::clone(&self.action),
        }
    }
}

/// A mouse button binding.
///
/// `target` is the `BarPosition` variant this binding responds to.  For
/// bindings that fire on any click of a given kind (e.g. any `Tag`, any
/// `WinTitle`) only the variant discriminant is matched — the inner value
/// (tag index, window handle) is passed to the action at call time so the
/// handler always knows the exact target.
pub struct Button {
    /// Which bar/screen region this binding applies to.
    pub target: BarPosition,
    /// Modifier mask.
    pub mask: u32,
    /// Mouse button.
    pub button: MouseButton,
    /// Action to execute when button is pressed.
    ///
    /// Arguments:
    /// * `&mut CoreCtx` — the window manager core context
    /// * `ButtonArg` — The exact bar region that was clicked, the mouse button that was pressed, and the x/y coordinates.
    pub action: Rc<dyn Fn(&mut CoreCtx, ButtonArg)>,
}

impl Button {
    /// Returns `true` when `pos` is the same variant as `self.target`.
    ///
    /// Inner values (tag index, window handle) are intentionally ignored
    /// during matching — they are passed to the action instead.
    pub fn matches(&self, pos: BarPosition) -> bool {
        std::mem::discriminant(&self.target) == std::mem::discriminant(&pos)
    }
}

impl Debug for Button {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Button")
            .field("target", &self.target)
            .field("mask", &self.mask)
            .field("button", &self.button)
            .field("action", &"<closure>")
            .finish()
    }
}

impl Clone for Button {
    fn clone(&self) -> Self {
        Self {
            target: self.target,
            mask: self.mask,
            button: self.button,
            action: Rc::clone(&self.action),
        }
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
