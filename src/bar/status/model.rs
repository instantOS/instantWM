use serde::{Deserialize, Serialize};

pub(crate) const TEXT_PADDING: i32 = 6;
pub(super) const DEFAULT_SEPARATOR_BLOCK_WIDTH: i32 = 9;

#[derive(Debug, Clone)]
pub(crate) enum StatusItem {
    Text(String),
    I3Block(I3Block),
}

#[derive(Debug, Clone)]
pub(crate) struct I3Block {
    pub full_text: String,
    pub short_text: Option<String>,
    pub color: Option<String>,
    pub background: Option<String>,
    pub border: Option<String>,
    pub border_top: i32,
    pub border_right: i32,
    pub border_bottom: i32,
    pub border_left: i32,
    pub min_width: Option<I3MinWidth>,
    pub align: I3Align,
    pub urgent: bool,
    pub separator: bool,
    pub separator_block_width: i32,
    pub name: Option<String>,
    pub instance: Option<String>,
    pub markup: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum I3MinWidth {
    Text(String),
    Pixels(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum I3Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct I3StatusLine {
    pub blocks: Vec<I3Block>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct I3BarHeader {
    pub version: Option<i32>,
    pub click_events: bool,
    pub stop_signal: Option<i32>,
    pub cont_signal: Option<i32>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StatusClickTarget {
    pub start_x: i32,
    pub end_x: i32,
    pub index: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ParsedStatus {
    pub items: Vec<StatusItem>,
    pub i3bar: Option<I3StatusLine>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct I3ClickEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    pub button: u8,
    pub x: i32,
    pub y: i32,
    pub relative_x: i32,
    pub relative_y: i32,
    pub width: i32,
    pub height: i32,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub modifiers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RawI3Block {
    #[serde(default)]
    pub full_text: String,
    #[serde(default)]
    pub short_text: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub border: Option<String>,
    #[serde(default)]
    pub border_top: Option<i32>,
    #[serde(default)]
    pub border_right: Option<i32>,
    #[serde(default)]
    pub border_bottom: Option<i32>,
    #[serde(default)]
    pub border_left: Option<i32>,
    #[serde(default)]
    pub min_width: Option<serde_json::Value>,
    #[serde(default)]
    pub align: Option<String>,
    #[serde(default)]
    pub urgent: bool,
    #[serde(default = "default_true")]
    pub separator: bool,
    #[serde(default)]
    pub separator_block_width: Option<i32>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub instance: Option<String>,
    #[serde(default)]
    pub markup: Option<String>,
}

fn default_true() -> bool {
    true
}
