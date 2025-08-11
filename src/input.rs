use crate::config::Config;
use crate::types::{Rectangle, WindowId};
use crate::window_manager::{TagId, WindowManager};
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, Event as InputEvent, InputBackend, KeyState,
    KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::input::{
    keyboard::{keysyms, FilterResult, KeysymHandle, ModifiersState},
    pointer::{AxisFrame, ButtonEvent, MotionEvent, PointerHandle},
};
use smithay::reexports::wayland_server::protocol::wl_pointer;
use smithay::utils::{Logical, Point};
use std::process::Command;
use std::sync::{Arc, Mutex};

pub struct InputHandler {
    pub window_manager: Arc<Mutex<WindowManager>>,
    pub config: Config,
    pub mod_key_pressed: bool,
    pub drag_state: Option<DragState>,
    pub resize_state: Option<ResizeState>,
}

#[derive(Debug, Clone)]
pub struct DragState {
    pub window_id: WindowId,
    pub start_pos: Point<i32, Logical>,
    pub start_geometry: Rectangle,
}

#[derive(Debug, Clone)]
pub struct ResizeState {
    pub window_id: WindowId,
    pub start_pos: Point<i32, Logical>,
    pub start_geometry: Rectangle,
    pub edge: ResizeEdge,
}

#[derive(Debug, Clone, Copy)]
pub enum ResizeEdge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl InputHandler {
    pub fn new(window_manager: Arc<Mutex<WindowManager>>, config: Config) -> Self {
        Self {
            window_manager,
            config,
            mod_key_pressed: false,
            drag_state: None,
            resize_state: None,
        }
    }

    pub fn handle_keyboard_event<B: InputBackend>(
        &mut self,
        event: impl KeyboardKeyEvent<B>,
    ) -> bool {
        let keycode = event.key_code();
        let state = event.state();
        let keysym = event.keysym();
        
        match state {
            KeyState::Pressed => {
                if keysym == keysyms::KEY_Super_L || keysym == keysyms::KEY_Super_R {
                    self.mod_key_pressed = true;
                    return false;
                }
                
                if self.mod_key_pressed {
                    self.handle_keybinding(keysym)
                } else {
                    false
                }
            }
            KeyState::Released => {
                if keysym == keysyms::KEY_Super_L || keysym == keysyms::KEY_Super_R {
                    self.mod_key_pressed = false;
                }
                false
            }
        }
    }

    fn handle_keybinding(&mut self, keysym: KeysymHandle) -> bool {
        let key_str = match keysym.raw() {
            keysyms::KEY_1 => "1",
            keysyms::KEY_2 => "2",
            keysyms::KEY_3 => "3",
            keysyms::KEY_4 => "4",
            keysyms::KEY_5 => "5",
            keysyms::KEY_6 => "6",
            keysyms::KEY_7 => "7",
            keysyms::KEY_8 => "8",
            keysyms::KEY_9 => "9",
            keysyms::KEY_Return => "Return",
            keysyms::KEY_q => "q",
            keysyms::KEY_f => "f",
            keysyms::KEY_space => "space",
            keysyms::KEY_h => "h",
            keysyms::KEY_j => "j",
            keysyms::KEY_k => "k",
            keysyms::KEY_l => "l",
            keysyms::KEY_r => "r",
            keysyms::KEY_e => "e",
            _ => return false,
        };
        
        let binding = format!("Mod4+{}", key_str);
        
        if let Some(action) = self.config.keybindings.get(&binding) {
            self.execute_action(action);
            true
        } else {
            false
        }
    }

    fn execute_action(&mut self, action: &str) {
        let parts: Vec<&str> = action.split_whitespace().collect();
        if parts.is_empty() {
            return;
        }
        
        match parts[0] {
            "spawn" => {
                if parts.len() > 1 {
                    let _ = Command::new(parts[1]).spawn();
                }
            }
            "close_window" => {
                let mut wm = self.window_manager.lock().unwrap();
                if let Some(focused) = wm.get_focused_window() {
                    let _ = wm.remove_window(focused);
                }
            }
            "toggle_floating" => {
                let mut wm = self.window_manager.lock().unwrap();
                if let Some(focused) = wm.get_focused_window() {
                    let _ = wm.toggle_floating(focused);
                }
            }
            "switch_tag" => {
                if parts.len() > 1 {
                    if let Ok(tag_num) = parts[1].parse::<usize>() {
                        let mut wm = self.window_manager.lock().unwrap();
                        let tag_id = TagId::new((tag_num - 1) as u32);
                        wm.switch_tag(tag_id);
                    }
                }
            }
            "move_to_tag" => {
                if parts.len() > 1 {
                    if let Ok(tag_num) = parts[1].parse::<usize>() {
                        let mut wm = self.window_manager.lock().unwrap();
                        if let Some(focused) = wm.get_focused_window() {
                            let tag_id = TagId::new((tag_num - 1) as u32);
                            let _ = wm.move_window_to_tag(focused, tag_id);
                        }
                    }
                }
            }
            "focus_left" => self.focus_direction(-1, 0),
            "focus_down" => self.focus_direction(0, 1),
            "focus_up" => self.focus_direction(0, -1),
            "focus_right" => self.focus_direction(1, 0),
            "move_left" => self.move_direction(-1, 0),
            "move_down" => self.move_direction(0, 1),
            "move_up" => self.move_direction(0, -1),
            "move_right" => self.move_direction(1, 0),
            "reload_config" => {
                // TODO: Reload config
            }
            "exit" => {
                // TODO: Graceful exit
            }
            _ => {}
        }
    }

    fn focus_direction(&mut self, dx: i32, dy: i32) {
        let wm = self.window_manager.lock().unwrap();
        if let Some(current) = wm.get_focused_window() {
            if let Some(window) = wm.get_window(current) {
                let current_center = (
                    window.geometry.x + window.geometry.width as i32 / 2,
                    window.geometry.y + window.geometry.height as i32 / 2,
                );
                
                let mut closest = None;
                let mut min_distance = i32::MAX;
                
                for &window_id in &wm.get_windows_for_tag(window.tag) {
                    if window_id == current {
                        continue;
                    }
                    
                    if let Some(other) = wm.get_window(window_id) {
                        let other_center = (
                            other.geometry.x + other.geometry.width as i32 / 2,
                            other.geometry.y + other.geometry.height as i32 / 2,
                        );
                        
                        let distance = if dx != 0 {
                            if (dx > 0 && other_center.0 > current_center.0) || 
                               (dx < 0 && other_center.0 < current_center.0) {
                                (other_center.0 - current_center.0).abs()
                            } else {
                                continue;
                            }
                        } else if dy != 0 {
                            if (dy > 0 && other_center.1 > current_center.1) || 
                               (dy < 0 && other_center.1 < current_center.1) {
                                (other_center.1 - current_center.1).abs()
                            } else {
                                continue;
                            }
                        } else {
                            continue;
                        };
                        
                        if distance < min_distance {
                            min_distance = distance;
                            closest = Some(window_id);
                        }
                    }
                }
                
                if let Some(closest_id) = closest {
                    drop(wm);
                    let mut wm = self.window_manager.lock().unwrap();
                    let _ = wm.focus_window(closest_id);
                }
            }
        }
    }

    fn move_direction(&mut self, dx: i32, dy: i32) {
        // TODO: Implement window movement
    }

    pub fn handle_pointer_motion<B: InputBackend>(
        &mut self,
        event: impl PointerMotionEvent<B>,
    ) -> bool {
        let pos = event.position();
        
        if let Some(drag) = &self.drag_state {
            let delta_x = pos.x as i32 - drag.start_pos.x;
            let delta_y = pos.y as i32 - drag.start_pos.y;
            
            let mut wm = self.window_manager.lock().unwrap();
            if let Some(window) = wm.get_window_mut(drag.window_id) {
                window.geometry.x = drag.start_geometry.x + delta_x;
                window.geometry.y = drag.start_geometry.y + delta_y;
            }
            return true;
        }
        
        if let Some(resize) = &self.resize_state {
            let delta_x = pos.x as i32 - resize.start_pos.x;
            let delta_y = pos.y as i32 - resize.start_pos.y;
            
            let mut wm = self.window_manager.lock().unwrap();
            if let Some(window) = wm.get_window_mut(resize.window_id) {
                match resize.edge {
                    ResizeEdge::Right => {
                        window.geometry.width = (resize.start_geometry.width as i32 + delta_x).max(100) as u32;
                    }
                    ResizeEdge::Bottom => {
                        window.geometry.height = (resize.start_geometry.height as i32 + delta_y).max(100) as u32;
                    }
                    ResizeEdge::BottomRight => {
                        window.geometry.width = (resize.start_geometry.width as i32 + delta_x).max(100) as u32;
                        window.geometry.height = (resize.start_geometry.height as i32 + delta_y).max(100) as u32;
                    }
                    _ => {}
                }
            }
            return true;
        }
        
        false
    }

    pub fn handle_pointer_button<B: InputBackend>(
        &mut self,
        event: impl PointerButtonEvent<B>,
    ) -> bool {
        let pos = event.position();
        let button = event.button();
        let state = event.state();
        
        match (button, state) {
            (0x110, wl_pointer::ButtonState::Pressed) => { // Left button
                self.start_drag_or_focus(pos)
            }
            (0x111, wl_pointer::ButtonState::Pressed) => { // Right button
                self.start_resize(pos)
            }
            (0x110, wl_pointer::ButtonState::Released) => {
                self.drag_state = None;
                self.resize_state = None;
                false
            }
            _ => false,
        }
    }

    fn start_drag_or_focus(&mut self, pos: Point<f64, Logical>) -> bool {
        let mut wm = self.window_manager.lock().unwrap();
        
        // Find window under cursor
        for &window_id in &wm.get_windows_for_tag(wm.current_tag) {
            if let Some(window) = wm.get_window(window_id) {
                let rect = &window.geometry;
                if rect.contains((pos.x as i32, pos.y as i32)) {
                    let _ = wm.focus_window(window_id);
                    
                    if window.floating {
                        self.drag_state = Some(DragState {
                            window_id,
                            start_pos: (pos.x as i32, pos.y as i32).into(),
                            start_geometry: rect.clone(),
                        });
                        return true;
                    }
                }
            }
        }
        
        false
    }

    fn start_resize(&mut self, pos: Point<f64, Logical>) -> bool {
        let wm = self.window_manager.lock().unwrap();
        
        for &window_id in &wm.get_windows_for_tag(wm.current_tag) {
            if let Some(window) = wm.get_window(window_id) {
                let rect = &window.geometry;
                if rect.contains((pos.x as i32, pos.y as i32)) {
                    if window.floating {
                        // Determine resize edge based on cursor position
                        let edge = self.determine_resize_edge(rect, (pos.x as i32, pos.y as i32));
                        
                        self.resize_state = Some(ResizeState {
                            window_id,
                            start_pos: (pos.x as i32, pos.y as i32).into(),
                            start_geometry: rect.clone(),
                            edge,
                        });
                        return true;
                    }
                }
            }
        }
        
        false
    }

    fn determine_resize_edge(&self, rect: &Rectangle, pos: (i32, i32)) -> ResizeEdge {
        let edge_threshold = 10;
        
        let right_edge = rect.x + rect.width as i32;
        let bottom_edge = rect.y + rect.height as i32;
        
        let near_right = (pos.0 - right_edge).abs() <= edge_threshold;
        let near_bottom = (pos.1 - bottom_edge).abs() <= edge_threshold;
        
        match (near_right, near_bottom) {
            (true, true) => ResizeEdge::BottomRight,
            (true, false) => ResizeEdge::Right,
            (false, true) => ResizeEdge::Bottom,
            _ => ResizeEdge::Right, // Default
        }
    }
}