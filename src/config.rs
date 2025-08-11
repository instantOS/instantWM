use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub appearance: Appearance,
    pub tags: Tags,
    pub layouts: Layouts,
    pub keybindings: Vec<Keybinding>,
}

#[derive(Deserialize)]
pub struct Appearance {
    pub border_px: u32,
    pub snap_px: u32,
    pub show_bar: bool,
    pub top_bar: bool,
    pub fonts: Vec<String>,
    pub colors: Colors,
}

#[derive(Deserialize)]
pub struct Colors {
    pub background: String,
    pub foreground: String,
    pub accent: String,
}

#[derive(Deserialize)]
pub struct Tags {
    pub names: Vec<String>,
    pub icons: Vec<String>,
}

#[derive(Deserialize)]
pub struct Layouts {
    pub mfact: f32,
    pub nmaster: u32,
    pub resize_hints: bool,
}

#[derive(Deserialize, Clone)]
pub struct Keybinding {
    pub modifiers: Vec<String>,
    pub key: String,
    pub action: Action,
}

#[derive(Deserialize, Clone)]
pub enum Action {
    SwitchToWorkspace(usize),
}
