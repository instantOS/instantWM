//! Input handling for InstantWM
//!
//! This module handles input events from keyboards, mice, and other input devices.
//! It processes raw input events and translates them into window manager actions.

use crate::types::Config;
use crate::window_manager::WindowManager;
use smithay::{
    backend::input::{KeyState, KeyboardKeyEvent, PointerButtonEvent},
    input::keyboard::ModifiersState,
    utils::Point,
};
use std::sync::{Arc, Mutex};
use tracing::debug;
use xkbcommon::xkb::Keysym;

/// Input handler for managing keyboard and pointer input
pub struct InputHandler {
    pub window_manager: Arc<Mutex<WindowManager>>,
    pub config: Config,
    pub pointer_location: Point<f64, smithay::utils::Logical>,
    pub modifiers_state: ModifiersState,
}

impl InputHandler {
    pub fn new(window_manager: Arc<Mutex<WindowManager>>, config: Config) -> Self {
        Self {
            window_manager,
            config,
            pointer_location: (0.0, 0.0).into(),
            modifiers_state: ModifiersState::default(),
        }
    }

    /// Handle a keyboard key event
    pub fn handle_keyboard_key(
        &mut self,
        _keycode: u32,
        key_state: KeyState,
        keysym: Keysym,
        modifiers: ModifiersState,
    ) {
        self.modifiers_state = modifiers;

        if key_state == KeyState::Pressed {
            debug!("Key pressed: {:?} (modifiers: {:?})", keysym, modifiers);

            if let Ok(mut wm) = self.window_manager.lock() {
                wm.handle_keybinding(keysym, modifiers);
            }
        }
    }

    /// Handle pointer motion
    pub fn handle_pointer_motion(&mut self, location: Point<f64, smithay::utils::Logical>) {
        self.pointer_location = location;
        debug!("Pointer motion: {:?}", location);
    }

    /// Handle pointer button press/release
    pub fn handle_pointer_button(
        &mut self,
        button: u32,
        state: smithay::backend::input::ButtonState,
    ) {
        debug!("Pointer button {} {:?}", button, state);

        // Handle window focus on click
        // This would be implemented with proper surface detection in a real compositor
    }

    /// Handle pointer axis (scroll wheel) - simplified version
    pub fn handle_pointer_axis(&mut self) {
        debug!("Pointer axis event");
    }

    /// Get current pointer location
    pub fn pointer_location(&self) -> Point<f64, smithay::utils::Logical> {
        self.pointer_location
    }

    /// Get current modifiers state
    pub fn modifiers_state(&self) -> ModifiersState {
        self.modifiers_state
    }

    /// Handle keyboard events - simplified version
    pub fn handle_keyboard_event<B: smithay::backend::input::InputBackend>(
        &mut self,
        event: &dyn KeyboardKeyEvent<B>,
    ) {
        debug!("Keyboard event: keycode {:?}", event.key_code());
    }

    /// Handle pointer motion events
    pub fn handle_pointer_motion_event<B: smithay::backend::input::InputBackend>(
        &mut self,
        _event: &dyn smithay::backend::input::PointerMotionEvent<B>,
    ) {
        self.handle_pointer_motion((0.0, 0.0).into()); // Simplified for now
    }

    /// Handle pointer button events
    pub fn handle_pointer_button_event<B: smithay::backend::input::InputBackend>(
        &mut self,
        event: &dyn PointerButtonEvent<B>,
    ) {
        if let Some(button) = event.button() {
            // Convert MouseButton to u32 - simplified mapping
            let button_code = match button {
                smithay::backend::input::MouseButton::Left => 1,
                smithay::backend::input::MouseButton::Right => 2,
                smithay::backend::input::MouseButton::Middle => 3,
                smithay::backend::input::MouseButton::Forward => 8,
                smithay::backend::input::MouseButton::Back => 9,
                _ => 0, // Default for any future variants
            };
            self.handle_pointer_button(button_code, event.state());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_handler_creation() {
        let config = Config::default();
        let screen_geometry =
            smithay::utils::Rectangle::from_size(smithay::utils::Size::from((1920, 1080)));
        let window_manager = Arc::new(Mutex::new(
            WindowManager::new(config.clone(), screen_geometry).unwrap(),
        ));

        let input_handler = InputHandler::new(window_manager, config);
        assert_eq!(input_handler.pointer_location, (0.0, 0.0).into());
    }

    #[test]
    fn test_pointer_motion() {
        let config = Config::default();
        let screen_geometry =
            smithay::utils::Rectangle::from_size(smithay::utils::Size::from((1920, 1080)));
        let window_manager = Arc::new(Mutex::new(
            WindowManager::new(config.clone(), screen_geometry).unwrap(),
        ));

        let mut input_handler = InputHandler::new(window_manager, config);
        let new_location = (100.0, 200.0).into();

        input_handler.handle_pointer_motion(new_location);
        assert_eq!(input_handler.pointer_location(), new_location);
    }
}
