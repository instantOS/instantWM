use crate::error::{InstantError, Result};
use crate::types::{Config, LayoutConfig, LayoutType, WindowRule};

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
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| InstantError::Config(format!("Failed to serialize config: {}", e)))?;
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

        // Terminal and application spawning
        keybindings.insert("Mod4+Return".to_string(), "spawn terminal".to_string());
        keybindings.insert("Mod4+d".to_string(), "spawn launcher".to_string());
        keybindings.insert("Mod4+Shift+Return".to_string(), "spawn browser".to_string());

        // Window management
        keybindings.insert("Mod4+q".to_string(), "close_window".to_string());
        keybindings.insert("Mod4+Shift+q".to_string(), "close_window".to_string());
        keybindings.insert("Mod4+f".to_string(), "toggle_floating".to_string());
        keybindings.insert("Mod4+Shift+f".to_string(), "toggle_fullscreen".to_string());
        keybindings.insert("Mod4+space".to_string(), "cycle_layout".to_string());

        // Tag management (1-9)
        for i in 1..=9 {
            keybindings.insert(format!("Mod4+{}", i), format!("switch_tag {}", i));
            keybindings.insert(format!("Mod4+Shift+{}", i), format!("move_to_tag {}", i));
        }

        // Focus navigation
        keybindings.insert("Mod4+h".to_string(), "focus_left".to_string());
        keybindings.insert("Mod4+j".to_string(), "focus_down".to_string());
        keybindings.insert("Mod4+k".to_string(), "focus_up".to_string());
        keybindings.insert("Mod4+l".to_string(), "focus_right".to_string());

        // Window movement
        keybindings.insert("Mod4+Shift+h".to_string(), "move_left".to_string());
        keybindings.insert("Mod4+Shift+j".to_string(), "move_down".to_string());
        keybindings.insert("Mod4+Shift+k".to_string(), "move_up".to_string());
        keybindings.insert("Mod4+Shift+l".to_string(), "move_right".to_string());

        // Master area management
        keybindings.insert(
            "Mod4+comma".to_string(),
            "increase_master_count".to_string(),
        );
        keybindings.insert(
            "Mod4+period".to_string(),
            "decrease_master_count".to_string(),
        );
        keybindings.insert("Mod4+Left".to_string(), "decrease_master".to_string());
        keybindings.insert("Mod4+Right".to_string(), "increase_master".to_string());

        // System controls
        keybindings.insert("Mod4+Shift+r".to_string(), "reload_config".to_string());
        keybindings.insert("Mod4+Shift+e".to_string(), "exit".to_string());
        keybindings.insert("Mod4+Shift+c".to_string(), "reload_config".to_string());

        // Additional instantWM specific bindings
        keybindings.insert("Mod4+Tab".to_string(), "focus_next".to_string());
        keybindings.insert("Mod4+Shift+Tab".to_string(), "focus_prev".to_string());
        keybindings.insert("Mod4+m".to_string(), "toggle_minimize".to_string());
        keybindings.insert("Mod4+o".to_string(), "toggle_sticky".to_string());

        // Screenshot and media keys
        keybindings.insert("Print".to_string(), "spawn grim".to_string());
        keybindings.insert(
            "Shift+Print".to_string(),
            "spawn grim -g \"$(slurp)\"".to_string(),
        );

        // Volume controls (XF86 keys)
        keybindings.insert(
            "XF86AudioRaiseVolume".to_string(),
            "spawn pactl set-sink-volume @DEFAULT_SINK@ +5%".to_string(),
        );
        keybindings.insert(
            "XF86AudioLowerVolume".to_string(),
            "spawn pactl set-sink-volume @DEFAULT_SINK@ -5%".to_string(),
        );
        keybindings.insert(
            "XF86AudioMute".to_string(),
            "spawn pactl set-sink-mute @DEFAULT_SINK@ toggle".to_string(),
        );

        // Brightness controls
        keybindings.insert(
            "XF86MonBrightnessUp".to_string(),
            "spawn brightnessctl set 5%+".to_string(),
        );
        keybindings.insert(
            "XF86MonBrightnessDown".to_string(),
            "spawn brightnessctl set 5%-".to_string(),
        );

        // Layout configurations
        let mut layouts = HashMap::new();
        layouts.insert(
            "tiling".to_string(),
            LayoutConfig {
                layout_type: LayoutType::Tiling,
                master_ratio: 0.55,
                master_count: 1,
            },
        );
        layouts.insert(
            "floating".to_string(),
            LayoutConfig {
                layout_type: LayoutType::Floating,
                master_ratio: 0.5,
                master_count: 1,
            },
        );
        layouts.insert(
            "monocle".to_string(),
            LayoutConfig {
                layout_type: LayoutType::Monocle,
                master_ratio: 1.0,
                master_count: 1,
            },
        );

        // Window rules (matching C version)
        let rules = vec![
            // Floating windows
            WindowRule {
                class: Some("Pavucontrol".to_string()),
                instance: None,
                title: None,
                floating: true,
                tag: None,
            },
            WindowRule {
                class: Some("Onboard".to_string()),
                instance: None,
                title: None,
                floating: true,
                tag: None,
            },
            WindowRule {
                class: Some("floatmenu".to_string()),
                instance: None,
                title: None,
                floating: true,
                tag: None,
            },
            WindowRule {
                class: Some("Welcome.py".to_string()),
                instance: None,
                title: None,
                floating: true,
                tag: None,
            },
            WindowRule {
                class: Some("Pamac-installer".to_string()),
                instance: None,
                title: None,
                floating: true,
                tag: None,
            },
            WindowRule {
                class: Some("xpad".to_string()),
                instance: None,
                title: None,
                floating: true,
                tag: None,
            },
            WindowRule {
                class: Some("Guake".to_string()),
                instance: None,
                title: None,
                floating: true,
                tag: None,
            },
            WindowRule {
                class: Some("instantfloat".to_string()),
                instance: None,
                title: None,
                floating: true,
                tag: None,
            },
            WindowRule {
                class: Some("Peek".to_string()),
                instance: None,
                title: None,
                floating: true,
                tag: None,
            },
            WindowRule {
                class: Some("kdeconnect.daemon".to_string()),
                instance: None,
                title: None,
                floating: true,
                tag: None,
            },
            WindowRule {
                class: Some("Panther".to_string()),
                instance: None,
                title: None,
                floating: true,
                tag: None,
            },
            WindowRule {
                class: Some("org-wellkord-globonote-Main".to_string()),
                instance: None,
                title: None,
                floating: true,
                tag: None,
            },
            // ROX-Filer should be tiled
            WindowRule {
                class: Some("ROX-Filer".to_string()),
                instance: None,
                title: None,
                floating: false,
                tag: None,
            },
        ];

        Self {
            general: crate::types::GeneralConfig {
                mod_key: "Mod4".to_string(),
                terminal: "alacritty".to_string(),
                browser: "firefox".to_string(),
                launcher: "rofi -show drun".to_string(),
                screenshot: "grim".to_string(),
            },
            tags: crate::types::TagsConfig {
                names: vec![
                    "1".to_string(),
                    "2".to_string(),
                    "3".to_string(),
                    "4".to_string(),
                    "5".to_string(),
                    "6".to_string(),
                    "7".to_string(),
                    "8".to_string(),
                    "9".to_string(),
                ],
                layouts: vec![
                    "tiling".to_string(),
                    "floating".to_string(),
                    "monocle".to_string(),
                ],
            },
            appearance: crate::types::AppearanceConfig {
                border_width: 3,
                border_focus: "#536DFE".to_string(),
                border_normal: "#384252".to_string(),
                gap_size: 5,
                inner_gap: 5,
                bar_height: 24,
                bar_background: "#121212".to_string(),
                bar_foreground: "#DFDFDF".to_string(),
                bar_font: "Inter-Regular".to_string(),
                bar_font_size: 12,
            },
            keybindings,
            layouts,
            rules,
        }
    }

    pub fn reload(&mut self) -> Result<()> {
        let new_config = Self::load()?;
        *self = new_config;
        Ok(())
    }

    pub fn get_layout(&self, name: &str) -> Option<&LayoutConfig> {
        self.layouts.get(name)
    }

    pub fn get_keybinding(&self, key: &str) -> Option<&String> {
        self.keybindings.get(key)
    }

    pub fn add_keybinding(&mut self, key: String, action: String) {
        self.keybindings.insert(key, action);
    }

    pub fn remove_keybinding(&mut self, key: &str) {
        self.keybindings.remove(key);
    }

    pub fn get_rules_for_window(&self, class: &str, title: &str) -> Vec<&WindowRule> {
        self.rules
            .iter()
            .filter(|rule| rule.matches(class, title))
            .collect()
    }

    /// Validate configuration and return any errors
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        // Validate tag names are not empty
        if self.tags.names.is_empty() {
            errors.push("At least one tag must be configured".to_string());
        }

        for (i, name) in self.tags.names.iter().enumerate() {
            if name.is_empty() {
                errors.push(format!("Tag {} has empty name", i));
            }
        }

        // Validate layout references
        for layout_name in &self.tags.layouts {
            if !self.layouts.contains_key(layout_name) {
                errors.push(format!(
                    "Layout '{}' referenced but not defined",
                    layout_name
                ));
            }
        }

        // Validate appearance settings
        if self.appearance.border_width > 10 {
            errors.push("Border width should not exceed 10 pixels".to_string());
        }

        if self.appearance.gap_size > 50 {
            errors.push("Gap size should not exceed 50 pixels".to_string());
        }

        if self.appearance.bar_height > 100 {
            errors.push("Bar height should not exceed 100 pixels".to_string());
        }

        // Validate keybindings format
        for (key, action) in &self.keybindings {
            if key.is_empty() {
                errors.push("Empty keybinding found".to_string());
            }
            if action.is_empty() {
                errors.push(format!("Empty action for keybinding '{}'", key));
            }
        }

        errors
    }
}
