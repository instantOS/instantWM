use crate::config::Config;
use crate::types::{
    BorderColorConfig, CloseButtonColorConfigs, StatusColorConfig, TagColorConfigs,
    WindowColorConfigs,
};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ConfigFile {
    pub fonts: Option<Vec<String>>,
    pub colors: ColorsFile,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ColorsFile {
    pub tag: TagColorConfigs,
    pub window: WindowColorConfigs,
    pub close_button: CloseButtonColorConfigs,
    pub border: BorderColorConfig,
    pub status: StatusColorConfig,
}

pub fn default_config_path() -> Option<PathBuf> {
    let config_dir = dirs::config_dir()?;
    Some(config_dir.join("instantwm").join("config.toml"))
}

pub fn load_config_toml(path: &Path) -> Result<ConfigFile, String> {
    let contents = fs::read_to_string(path).map_err(|err| err.to_string())?;
    toml::from_str(&contents).map_err(|err| err.to_string())
}

impl Config {
    pub fn apply_overrides(&mut self, file: ConfigFile) {
        if let Some(fonts) = file.fonts {
            if !fonts.is_empty() {
                self.fonts = fonts;
            }
        }

        self.tag_colors = file.colors.tag;
        self.windowcolors = file.colors.window;
        self.closebuttoncolors = file.colors.close_button;
        self.bordercolors = file.colors.border;
        self.statusbarcolors = file.colors.status;
    }
}

pub fn apply_config_overrides(cfg: &mut Config) -> Result<(), String> {
    let Some(path) = default_config_path() else {
        return Ok(());
    };

    if !path.exists() {
        return Ok(());
    }

    let parsed = load_config_toml(&path)?;
    cfg.apply_overrides(parsed);
    Ok(())
}
