//! Window system and resource types.
//!
//! Types for keyboard bindings, mouse buttons, X commands, and X resources.

use std::fmt::Debug;
use std::rc::Rc;

use x11rb::protocol::xproto::Window;

use crate::contexts::WmCtx;
use crate::types::input::BarPosition;
use crate::types::input::MouseButton;

/// A keyboard binding.
pub struct Key {
    /// Modifier mask (e.g., Mod1Mask, ControlMask).
    pub mod_mask: u32,
    /// Keysym value.
    pub keysym: u32,
    /// Action to execute when key is pressed.
    pub action: Rc<Box<dyn Fn(&mut WmCtx)>>,
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
    /// * `&mut WmCtx` — the window manager context
    /// * `BarPosition` — the exact bar region that was clicked (tag index,
    ///   window handle, etc.)
    /// * `i32, i32` — `root_x`, `root_y` from the `ButtonPressEvent`,
    ///   available without any extra X11 round-trip.  Drag handlers use
    ///   these as their initial anchor instead of calling `get_root_ptr`.
    ///   Handlers that don't need the coordinates use `_`.
    pub action: Rc<Box<dyn Fn(&mut WmCtx, BarPosition, i32, i32)>>,
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

/// An X command that can be invoked by name.
#[derive(Debug, Clone)]
pub struct XCommand {
    /// Command name.
    pub cmd: &'static str,
    /// Action function taking context and argument string.
    pub action: fn(&mut WmCtx, &str),
}

/// X resource value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceType {
    /// String value.
    String,
    /// Integer value.
    Integer,
    /// Float value.
    Float,
}

/// X resource preference definition.
#[derive(Debug, Clone)]
pub struct ResourcePref {
    /// Resource name.
    pub name: &'static str,
    /// Resource type.
    pub rtype: ResourceType,
}

/// System tray state.
#[derive(Debug, Clone)]
pub struct Systray {
    /// Tray window handle.
    pub win: Window,
    /// List of tray icon windows.
    pub icons: Vec<Window>,
}
