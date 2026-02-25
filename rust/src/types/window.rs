//! Window system and resource types.
//!
//! Types for keyboard bindings, mouse buttons, X commands, and X resources.

use std::fmt::Debug;
use std::rc::Rc;

use x11rb::protocol::xproto::Window;

use crate::contexts::WmCtx;
use crate::types::input::Click;
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
pub struct Button {
    /// Click target area.
    pub click: Click,
    /// Modifier mask.
    pub mask: u32,
    /// Mouse button.
    pub button: MouseButton,
    /// Action to execute when button is pressed.
    pub action: Rc<Box<dyn Fn(&mut WmCtx)>>,
}

impl Debug for Button {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Button")
            .field("click", &self.click)
            .field("mask", &self.mask)
            .field("button", &self.button)
            .field("action", &"<closure>")
            .finish()
    }
}

impl Clone for Button {
    fn clone(&self) -> Self {
        Self {
            click: self.click,
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
