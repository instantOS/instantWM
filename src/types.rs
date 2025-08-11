use serde::{Deserialize, Serialize};
use slotmap::{new_key_type, SlotMap};
use std::collections::HashMap;

new_key_type! {
    pub struct WindowId;
    pub struct TagId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutType {
    Tiling,
    Floating,
    Monocle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutConfig {
    pub layout_type: LayoutType,
    pub master_ratio: f32,
    pub master_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: TagId,
    pub name: String,
    pub layout: LayoutConfig,
    pub windows: Vec<WindowId>,
    pub focused_window: Option<WindowId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowState {
    pub id: WindowId,
    pub title: String,
    pub class: String,
    pub floating: bool,
    pub minimized: bool,
    pub fullscreen: bool,
    pub tag: TagId,
    pub geometry: Rectangle,
    pub requested_geometry: Rectangle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rectangle {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keybinding {
    pub keys: Vec<String>,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub general: GeneralConfig,
    pub tags: TagsConfig,
    pub appearance: AppearanceConfig,
    pub keybindings: HashMap<String, String>,
    pub layouts: HashMap<String, LayoutConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub mod_key: String,
    pub terminal: String,
    pub browser: String,
    pub launcher: String,
    pub screenshot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagsConfig {
    pub names: Vec<String>,
    pub layouts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceConfig {
    pub border_width: u32,
    pub border_focus: String,
    pub border_normal: String,
    pub gap_size: u32,
    pub inner_gap: u32,
    pub bar_height: u32,
    pub bar_background: String,
    pub bar_foreground: String,
    pub bar_font: String,
    pub bar_font_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcMessage {
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub success: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

impl Rectangle {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    pub fn contains(&self, point: (i32, i32)) -> bool {
        point.0 >= self.x && 
        point.0 < self.x + self.width as i32 && 
        point.1 >= self.y && 
        point.1 < self.y + self.height as i32
    }

    pub fn intersects(&self, other: &Rectangle) -> bool {
        self.x < other.x + other.width as i32 &&
        self.x + self.width as i32 > other.x &&
        self.y < other.y + other.height as i32 &&
        self.y + self.height as i32 > other.y
    }
}