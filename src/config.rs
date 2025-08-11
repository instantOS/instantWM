use crate::error::{InstantError, Result};
use crate::types::{Config, LayoutConfig, LayoutType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = Self::get_config_path()?;
        
        if !config_path.exists() {
            let default_config = Self::default();
            default_config.save()?;
            return Ok(default_config);
        }

        let content = fs::read_to_string(&config_path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let config_path = Self::get_config_path()?;
        let content = toml::to_string_pretty(self)?;
        fs::write(config_path, content)?;
        Ok(())
    }

    fn get_config_path() -> Result<PathBuf> {
        let mut path = dirs::config_dir()
            .ok_or_else(|| InstantError::Config("Could not find config directory".to_string()))?;
        path.push("instantwm");
        fs::create_dir_all(&path)?;
        path.push("config.toml");
        Ok(path)
    }

    pub fn default() -> Self {
        let mut keybindings = HashMap::new();
        
        // Window management
        keybindings.insert("Mod4+Return".to_string(), "spawn terminal".to_string());
        keybindings.insert("Mod4+q".to_string(), "close_window".to_string());
        keybindings.insert("Mod4+f".to_string(), "toggle_floating".to_string());
        keybindings.insert("Mod4+space".to_string(), "toggle_layout".to_string());
        
        // Tag management
        for i in 1..=9 {
            keybindings.insert(format!("Mod4+{}", i), format!("switch_tag {}", i));
            keybindings.insert(format!("Mod4+Shift+{}", i), format!("move_to_tag {}", i));
        }
        
        // Navigation
        keybindings.insert("Mod4+h".to_string(), "focus_left".to_string());
        keybindings.insert("Mod4+j".to_string(), "focus_down".to_string());
        keybindings.insert("Mod4+k".to_string(), "focus_up".to_string());
        keybindings.insert("Mod4+l".to_string(), "focus_right".to_string());
        
        // Movement
        keybindings.insert("Mod4+Shift+h".to_string(), "move_left".to_string());
        keybindings.insert("Mod4+Shift+j".to_string(), "move_down".to_string());
        keybindings.insert("Mod4+Shift+k".to_string(), "move_up".to_string());
        keybindings.insert("Mod4+Shift+l".to_string(), "move_right".to_string());
        
        // System
        keybindings.insert("Mod4+Shift+r".to_string(), "reload_config".to_string());
        keybindings.insert("Mod4+Shift+e".to_string(), "exit".to_string());

        let mut layouts = HashMap::new();
        layouts.insert("tiling".to_string(), LayoutConfig {
            layout_type: LayoutType::Tiling,
            master_ratio: 0.6,
            master_count: 1,
        });
        layouts.insert("floating".to_string(), LayoutConfig {
            layout_type: LayoutType::Floating,
            master_ratio: 0.5,
            master_count: 1,
        });
        layouts.insert("monocle".to_string(), LayoutConfig {
            layout_type: LayoutType::Monocle,
            master_ratio: 1.0,
            master_count: 1,
        });

        Self {
            general: crate::types::GeneralConfig {
                mod_key: "Mod4".to_string(),
                terminal: "alacritty".to_string(),
                browser: "firefox".to_string(),
                launcher: "rofi -show drun".to_string(),
                screenshot: "grim".to_string(),
            },
            tags: crate::types::TagsConfig {
                names: (1..=9).map(|i| i.to_string()).collect(),
                layouts: vec!["tiling".to_string(), "floating".to_string(), "monocle".to_string()],
            },
            appearance: crate::types::AppearanceConfig {
                border_width: 2,
                border_focus: "#ff5555".to_string(),
                border_normal: "#333333".to_string(),
                gap_size: 5,
                inner_gap: 0,
                bar_height: 24,
                bar_background: "#222222".to_string(),
                bar_foreground: "#ffffff".to_string(),
                bar_font: "monospace".to_string(),
                bar_font_size: 12,
            },
            keybindings,
            layouts,
        }
    }
}