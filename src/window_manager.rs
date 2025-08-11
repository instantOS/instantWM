use crate::error::Result;
use crate::types::{Config, LayoutType};
use smithay::{
    desktop::Window,
    input::keyboard::ModifiersState,
    utils::{Point, Rectangle as SmithayRectangle},
};
use std::collections::HashMap;
use tracing::{debug, info, warn};
use xkbcommon::xkb::Keysym;

#[derive(Debug, Clone)]
pub struct Tag {
    pub id: u32,
    pub name: String,
    pub layout: LayoutType,
    pub windows: Vec<Window>,
    pub focused_window: Option<Window>,
    pub master_count: usize,
    pub master_ratio: f32,
}

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub window: Window,
    pub floating: bool,
    pub minimized: bool,
    pub fullscreen: bool,
    pub tag: u32,
    pub original_geometry: Option<SmithayRectangle<i32, smithay::utils::Logical>>,
}

pub struct WindowManager {
    pub tags: Vec<Tag>,
    pub current_tag: u32,
    pub window_info: HashMap<Window, WindowInfo>,
    pub config: Config,
    pub screen_geometry: SmithayRectangle<i32, smithay::utils::Logical>,
    pub next_window_offset: Point<i32, smithay::utils::Logical>,
}

impl WindowManager {
    pub fn new(
        config: Config,
        screen_geometry: SmithayRectangle<i32, smithay::utils::Logical>,
    ) -> Result<Self> {
        let mut tags = Vec::new();

        // Create tags from config
        for (i, name) in config.tags.names.iter().enumerate() {
            let layout = if !config.tags.layouts.is_empty() {
                match config.tags.layouts[i % config.tags.layouts.len()].as_str() {
                    "floating" => LayoutType::Floating,
                    "monocle" => LayoutType::Monocle,
                    _ => LayoutType::Tiling,
                }
            } else {
                LayoutType::Tiling
            };

            let tag = Tag {
                id: i as u32,
                name: name.clone(),
                layout,
                windows: Vec::new(),
                focused_window: None,
                master_count: 1,
                master_ratio: 0.6,
            };
            tags.push(tag);
        }

        Ok(Self {
            tags,
            current_tag: 0,
            window_info: HashMap::new(),
            config,
            screen_geometry,
            next_window_offset: Point::from((50, 50)),
        })
    }

    pub fn manage_window(&mut self, window: Window) {
        info!("Managing window");

        // Apply window rules
        let mut floating = false;
        let mut target_tag = self.current_tag;

        // Apply window rules based on window properties (simplified for now)
        // TODO: Implement proper app_id and title matching when available
        for rule in &self.config.rules {
            // For now, just apply default floating rules for common cases
            if rule.class.as_ref().map_or(false, |class| {
                class == "Pavucontrol" || class == "floatmenu" || class == "Guake"
            }) {
                floating = rule.floating;
                if let Some(rule_tag) = rule.tag {
                    if rule_tag < self.tags.len() as u32 {
                        target_tag = rule_tag;
                    }
                }
                break;
            }
        }

        let info = WindowInfo {
            window: window.clone(),
            floating,
            minimized: false,
            fullscreen: false,
            tag: target_tag,
            original_geometry: None,
        };

        self.window_info.insert(window.clone(), info);
        self.tags[target_tag as usize].windows.push(window.clone());

        // Focus the new window if it's on the current tag
        if target_tag == self.current_tag {
            self.tags[target_tag as usize].focused_window = Some(window);
        }

        debug!("Window managed on tag {}", target_tag);
    }

    pub fn unmanage_window(&mut self, window: &Window) {
        info!("Unmanaging window");

        if let Some(info) = self.window_info.remove(window) {
            let tag = &mut self.tags[info.tag as usize];
            tag.windows.retain(|w| w != window);

            // Update focus if this was the focused window
            if tag.focused_window.as_ref() == Some(window) {
                tag.focused_window = tag.windows.last().cloned();
            }
        }
    }

    pub fn switch_to_tag(&mut self, tag_id: u32) {
        if tag_id < self.tags.len() as u32 && tag_id != self.current_tag {
            info!(
                "Switching to tag {}: {}",
                tag_id, self.tags[tag_id as usize].name
            );
            self.current_tag = tag_id;
        }
    }

    pub fn move_window_to_tag(&mut self, window: &Window, tag_id: u32) {
        if tag_id >= self.tags.len() as u32 {
            return;
        }

        if let Some(info) = self.window_info.get_mut(window) {
            let old_tag = info.tag;
            info.tag = tag_id;

            // Remove from old tag
            self.tags[old_tag as usize].windows.retain(|w| w != window);
            if self.tags[old_tag as usize].focused_window.as_ref() == Some(window) {
                self.tags[old_tag as usize].focused_window =
                    self.tags[old_tag as usize].windows.last().cloned();
            }

            // Add to new tag
            self.tags[tag_id as usize].windows.push(window.clone());
            self.tags[tag_id as usize].focused_window = Some(window.clone());

            info!("Moved window from tag {} to tag {}", old_tag, tag_id);
        }
    }

    pub fn toggle_floating(&mut self, window: &Window) {
        if let Some(info) = self.window_info.get_mut(window) {
            info.floating = !info.floating;

            if info.floating {
                info!("Window is now floating");
            } else {
                info!("Window is now tiling");
            }
        }
    }

    pub fn toggle_fullscreen(&mut self, window: &Window) {
        if let Some(info) = self.window_info.get_mut(window) {
            info.fullscreen = !info.fullscreen;

            if info.fullscreen {
                info.original_geometry = Some(window.geometry());
                info!("Window is now fullscreen");
            } else {
                info!("Window exited fullscreen");
            }
        }
    }

    pub fn focus_window(&mut self, window: &Window) {
        if let Some(info) = self.window_info.get(window) {
            self.tags[info.tag as usize].focused_window = Some(window.clone());
            debug!("Focused window");
        }
    }

    pub fn close_focused_window(&mut self) {
        if let Some(window) = self.get_focused_window() {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_close();
            }
            info!("Closing focused window");
        }
    }

    pub fn get_focused_window(&self) -> Option<Window> {
        self.tags[self.current_tag as usize].focused_window.clone()
    }

    pub fn get_current_tag(&self) -> &Tag {
        &self.tags[self.current_tag as usize]
    }

    pub fn get_current_tag_mut(&mut self) -> &mut Tag {
        &mut self.tags[self.current_tag as usize]
    }

    pub fn current_layout(&self) -> LayoutType {
        self.tags[self.current_tag as usize].layout
    }

    pub fn set_layout(&mut self, layout: LayoutType) {
        self.tags[self.current_tag as usize].layout = layout;
        info!(
            "Changed layout to {:?} for tag {}",
            layout, self.current_tag
        );
    }

    pub fn cycle_layout(&mut self) {
        let current = self.tags[self.current_tag as usize].layout;
        let new_layout = match current {
            LayoutType::Tiling => LayoutType::Floating,
            LayoutType::Floating => LayoutType::Monocle,
            LayoutType::Monocle => LayoutType::Tiling,
        };
        self.set_layout(new_layout);
    }

    pub fn adjust_master_ratio(&mut self, delta: f32) {
        let tag = &mut self.tags[self.current_tag as usize];
        tag.master_ratio = (tag.master_ratio + delta).clamp(0.1, 0.9);
        debug!("Adjusted master ratio to {}", tag.master_ratio);
    }

    pub fn adjust_master_count(&mut self, delta: i32) {
        let tag = &mut self.tags[self.current_tag as usize];
        let new_count = (tag.master_count as i32 + delta).max(1) as usize;
        tag.master_count = new_count.min(tag.windows.len());
        debug!("Adjusted master count to {}", tag.master_count);
    }

    pub fn focus_next(&mut self) {
        let tag = &mut self.tags[self.current_tag as usize];
        if tag.windows.is_empty() {
            return;
        }

        if let Some(current) = &tag.focused_window {
            if let Some(pos) = tag.windows.iter().position(|w| w == current) {
                let next_pos = (pos + 1) % tag.windows.len();
                tag.focused_window = Some(tag.windows[next_pos].clone());
            }
        } else {
            tag.focused_window = tag.windows.first().cloned();
        }

        if let Some(_window) = &tag.focused_window {
            debug!("Focused next window");
        }
    }

    pub fn focus_prev(&mut self) {
        let tag = &mut self.tags[self.current_tag as usize];
        if tag.windows.is_empty() {
            return;
        }

        if let Some(current) = &tag.focused_window {
            if let Some(pos) = tag.windows.iter().position(|w| w == current) {
                let prev_pos = if pos == 0 {
                    tag.windows.len() - 1
                } else {
                    pos - 1
                };
                tag.focused_window = Some(tag.windows[prev_pos].clone());
            }
        } else {
            tag.focused_window = tag.windows.last().cloned();
        }

        if let Some(_window) = &tag.focused_window {
            debug!("Focused previous window");
        }
    }

    pub fn move_window_up(&mut self) {
        let tag = &mut self.tags[self.current_tag as usize];
        if let Some(focused) = &tag.focused_window {
            if let Some(pos) = tag.windows.iter().position(|w| w == focused) {
                if pos > 0 {
                    tag.windows.swap(pos, pos - 1);
                    debug!("Moved window up in stack");
                }
            }
        }
    }

    pub fn move_window_down(&mut self) {
        let tag = &mut self.tags[self.current_tag as usize];
        if let Some(focused) = &tag.focused_window {
            if let Some(pos) = tag.windows.iter().position(|w| w == focused) {
                if pos < tag.windows.len() - 1 {
                    tag.windows.swap(pos, pos + 1);
                    debug!("Moved window down in stack");
                }
            }
        }
    }

    pub fn get_windows_for_current_tag(&self) -> Vec<Window> {
        self.tags[self.current_tag as usize].windows.clone()
    }

    pub fn get_tiled_windows_for_current_tag(&self) -> Vec<Window> {
        self.tags[self.current_tag as usize]
            .windows
            .iter()
            .filter(|w| {
                self.window_info.get(w).map_or(true, |info| {
                    !info.floating && !info.minimized && !info.fullscreen
                })
            })
            .cloned()
            .collect()
    }

    pub fn is_window_floating(&self, window: &Window) -> bool {
        self.window_info
            .get(window)
            .map_or(false, |info| info.floating)
    }

    pub fn handle_keybinding(&mut self, keysym: Keysym, modifiers: ModifiersState) {
        let key_string = format_keybinding(keysym, modifiers);
        debug!("Checking keybinding: {}", key_string);

        if let Some(action) = self.config.keybindings.get(&key_string).cloned() {
            debug!("Executing keybinding: {} -> {}", key_string, action);
            self.execute_action(&action);
        } else {
            debug!("No action found for keybinding: {}", key_string);
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
                    let command = match parts[1] {
                        "terminal" => &self.config.general.terminal,
                        "browser" => &self.config.general.browser,
                        "launcher" => &self.config.general.launcher,
                        _ => parts[1],
                    };
                    spawn_command(command);
                }
            }
            "close_window" => self.close_focused_window(),
            "toggle_floating" => {
                if let Some(window) = self.get_focused_window() {
                    self.toggle_floating(&window);
                }
            }
            "toggle_fullscreen" => {
                if let Some(window) = self.get_focused_window() {
                    self.toggle_fullscreen(&window);
                }
            }
            "toggle_layout" | "cycle_layout" => self.cycle_layout(),
            "switch_tag" => {
                if parts.len() > 1 {
                    if let Ok(tag_id) = parts[1].parse::<u32>() {
                        if tag_id > 0 {
                            self.switch_to_tag(tag_id - 1); // Convert from 1-based to 0-based
                        }
                    }
                }
            }
            "move_to_tag" => {
                if parts.len() > 1 {
                    if let Ok(tag_id) = parts[1].parse::<u32>() {
                        if tag_id > 0 {
                            if let Some(window) = self.get_focused_window() {
                                self.move_window_to_tag(&window, tag_id - 1); // Convert from 1-based to 0-based
                            }
                        }
                    }
                }
            }
            "focus_left" | "focus_up" => self.focus_prev(),
            "focus_right" | "focus_down" => self.focus_next(),
            "move_left" | "move_up" => self.move_window_up(),
            "move_right" | "move_down" => self.move_window_down(),
            "increase_master" => self.adjust_master_ratio(0.05),
            "decrease_master" => self.adjust_master_ratio(-0.05),
            "increase_master_count" => self.adjust_master_count(1),
            "decrease_master_count" => self.adjust_master_count(-1),
            "reload_config" => {
                info!("Reloading configuration...");
                // This would reload the config in practice
            }
            "exit" => {
                info!("Exit command received");
                std::process::exit(0);
            }
            _ => {
                warn!("Unknown action: {}", action);
            }
        }
    }
}

fn format_keybinding(keysym: Keysym, modifiers: ModifiersState) -> String {
    let mut parts = Vec::new();

    if modifiers.ctrl {
        parts.push("Ctrl");
    }
    if modifiers.alt {
        parts.push("Alt");
    }
    if modifiers.shift {
        parts.push("Shift");
    }
    if modifiers.logo {
        parts.push("Mod4"); // Super/Windows key
    }

    // Convert keysym to string
    let key_name = match keysym.raw() {
        xkbcommon::xkb::keysyms::KEY_Return => "Return",
        xkbcommon::xkb::keysyms::KEY_space => "space",
        xkbcommon::xkb::keysyms::KEY_Tab => "Tab",
        xkbcommon::xkb::keysyms::KEY_h => "h",
        xkbcommon::xkb::keysyms::KEY_j => "j",
        xkbcommon::xkb::keysyms::KEY_k => "k",
        xkbcommon::xkb::keysyms::KEY_l => "l",
        xkbcommon::xkb::keysyms::KEY_q => "q",
        xkbcommon::xkb::keysyms::KEY_d => "d",
        xkbcommon::xkb::keysyms::KEY_f => "f",
        xkbcommon::xkb::keysyms::KEY_m => "m",
        xkbcommon::xkb::keysyms::KEY_o => "o",
        xkbcommon::xkb::keysyms::KEY_r => "r",
        xkbcommon::xkb::keysyms::KEY_e => "e",
        xkbcommon::xkb::keysyms::KEY_c => "c",
        xkbcommon::xkb::keysyms::KEY_comma => "comma",
        xkbcommon::xkb::keysyms::KEY_period => "period",
        xkbcommon::xkb::keysyms::KEY_Left => "Left",
        xkbcommon::xkb::keysyms::KEY_Right => "Right",
        xkbcommon::xkb::keysyms::KEY_1 => "1",
        xkbcommon::xkb::keysyms::KEY_2 => "2",
        xkbcommon::xkb::keysyms::KEY_3 => "3",
        xkbcommon::xkb::keysyms::KEY_4 => "4",
        xkbcommon::xkb::keysyms::KEY_5 => "5",
        xkbcommon::xkb::keysyms::KEY_6 => "6",
        xkbcommon::xkb::keysyms::KEY_7 => "7",
        xkbcommon::xkb::keysyms::KEY_8 => "8",
        xkbcommon::xkb::keysyms::KEY_9 => "9",
        xkbcommon::xkb::keysyms::KEY_0 => "0",
        xkbcommon::xkb::keysyms::KEY_Print => "Print",
        xkbcommon::xkb::keysyms::KEY_F1 => "F1",
        xkbcommon::xkb::keysyms::KEY_F2 => "F2",
        xkbcommon::xkb::keysyms::KEY_F3 => "F3",
        xkbcommon::xkb::keysyms::KEY_F4 => "F4",
        xkbcommon::xkb::keysyms::KEY_F5 => "F5",
        xkbcommon::xkb::keysyms::KEY_F6 => "F6",
        xkbcommon::xkb::keysyms::KEY_F7 => "F7",
        xkbcommon::xkb::keysyms::KEY_F8 => "F8",
        xkbcommon::xkb::keysyms::KEY_F9 => "F9",
        xkbcommon::xkb::keysyms::KEY_F10 => "F10",
        xkbcommon::xkb::keysyms::KEY_F11 => "F11",
        xkbcommon::xkb::keysyms::KEY_F12 => "F12",
        xkbcommon::xkb::keysyms::KEY_Escape => "Escape",

        xkbcommon::xkb::keysyms::KEY_Up => "Up",
        xkbcommon::xkb::keysyms::KEY_Down => "Down",
        _ => "Unknown",
    };

    parts.push(key_name);
    parts.join("+")
}

fn spawn_command(command: &str) {
    use std::process::Command;

    let args: Vec<&str> = command.split_whitespace().collect();
    if args.is_empty() {
        return;
    }

    match Command::new(args[0]).args(&args[1..]).spawn() {
        Ok(_) => debug!("Spawned command: {}", command),
        Err(e) => warn!("Failed to spawn command '{}': {}", command, e),
    }
}
