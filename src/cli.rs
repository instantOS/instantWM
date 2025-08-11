use crate::config::Config;
use crate::types::{LayoutType, Rectangle};
use crate::window_manager::{TagId, WindowManager};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "instantctl")]
#[command(about = "Control instantWM from the command line")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Switch to a specific tag
    Tag {
        /// Tag number (1-9)
        number: usize,
    },
    
    /// Move focused window to a specific tag
    MoveToTag {
        /// Tag number (1-9)
        number: usize,
    },
    
    /// Toggle floating mode for focused window
    ToggleFloating,
    
    /// Close focused window
    Close,
    
    /// Spawn a new application
    Spawn {
        /// Command to execute
        command: String,
        /// Arguments to pass
        args: Vec<String>,
    },
    
    /// Get current state information
    Get {
        #[command(subcommand)]
        what: GetCommands,
    },
    
    /// Set configuration values
    Set {
        #[command(subcommand)]
        what: SetCommands,
    },
    
    /// Reload configuration
    Reload,
    
    /// Exit instantWM
    Exit,
}

#[derive(Subcommand)]
pub enum GetCommands {
    /// Get current tag
    Tag,
    /// Get list of windows on current tag
    Windows,
    /// Get focused window information
    Focused,
    /// Get configuration
    Config,
    /// Get version information
    Version,
}

#[derive(Subcommand)]
pub enum SetCommands {
    /// Set layout for current tag
    Layout {
        layout: String,
    },
    /// Set gap size
    Gap {
        size: u32,
    },
    /// Set border width
    Border {
        width: u32,
    },
}

#[derive(Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub class: String,
    pub geometry: Rectangle,
    pub floating: bool,
    pub tag: usize,
}

#[derive(Serialize, Deserialize)]
pub struct TagInfo {
    pub number: usize,
    pub name: String,
    pub active: bool,
    pub windows: Vec<WindowInfo>,
}

#[derive(Serialize, Deserialize)]
pub struct StateInfo {
    pub current_tag: usize,
    pub tags: Vec<TagInfo>,
    pub focused_window: Option<WindowInfo>,
}

pub struct CliHandler {
    pub window_manager: Arc<Mutex<WindowManager>>,
    pub config: Config,
}

impl CliHandler {
    pub fn new(window_manager: Arc<Mutex<WindowManager>>, config: Config) -> Self {
        Self {
            window_manager,
            config,
        }
    }

    pub fn handle_command(&self, command: Commands) -> Result<String, String> {
        match command {
            Commands::Tag { number } => {
                let mut wm = self.window_manager.lock().unwrap();
                if number > 0 && number <= wm.tags.len() {
                    let tag_id = TagId::new((number - 1) as u32);
                    wm.switch_tag(tag_id);
                    Ok(format!("Switched to tag {}", number))
                } else {
                    Err(format!("Invalid tag number: {}", number))
                }
            }
            
            Commands::MoveToTag { number } => {
                let mut wm = self.window_manager.lock().unwrap();
                if number > 0 && number <= wm.tags.len() {
                    if let Some(focused) = wm.get_focused_window() {
                        let tag_id = TagId::new((number - 1) as u32);
                        let _ = wm.move_window_to_tag(focused, tag_id);
                        Ok(format!("Moved window to tag {}", number))
                    } else {
                        Err("No focused window".to_string())
                    }
                } else {
                    Err(format!("Invalid tag number: {}", number))
                }
            }
            
            Commands::ToggleFloating => {
                let mut wm = self.window_manager.lock().unwrap();
                if let Some(focused) = wm.get_focused_window() {
                    let _ = wm.toggle_floating(focused);
                    Ok("Toggled floating mode".to_string())
                } else {
                    Err("No focused window".to_string())
                }
            }
            
            Commands::Close => {
                let mut wm = self.window_manager.lock().unwrap();
                if let Some(focused) = wm.get_focused_window() {
                    let _ = wm.remove_window(focused);
                    Ok("Closed window".to_string())
                } else {
                    Err("No focused window".to_string())
                }
            }
            
            Commands::Spawn { command, args } => {
                use std::process::Command as ProcessCommand;
                let _ = ProcessCommand::new(&command).args(&args).spawn();
                Ok(format!("Spawned: {} {}", command, args.join(" ")))
            }
            
            Commands::Get { what } => {
                let wm = self.window_manager.lock().unwrap();
                match what {
                    GetCommands::Tag => {
                        let current_tag = wm.current_tag;
                        Ok(format!("Current tag: {}", current_tag.as_u32() + 1))
                    }
                    
                    GetCommands::Windows => {
                        let mut windows = Vec::new();
                        for &window_id in &wm.get_windows_for_tag(wm.current_tag) {
                            if let Some(window) = wm.get_window(window_id) {
                                windows.push(WindowInfo {
                                    id: window_id.as_u32(),
                                    title: window.title.clone(),
                                    class: window.class.clone(),
                                    geometry: window.geometry.clone(),
                                    floating: window.floating,
                                    tag: wm.current_tag.as_u32() as usize + 1,
                                });
                            }
                        }
                        serde_json::to_string_pretty(&windows)
                            .map_err(|e| e.to_string())
                    }
                    
                    GetCommands::Focused => {
                        if let Some(focused) = wm.get_focused_window() {
                            if let Some(window) = wm.get_window(focused) {
                                let info = WindowInfo {
                                    id: focused.as_u32(),
                                    title: window.title.clone(),
                                    class: window.class.clone(),
                                    geometry: window.geometry.clone(),
                                    floating: window.floating,
                                    tag: window.tag.as_u32() as usize + 1,
                                };
                                serde_json::to_string_pretty(&info)
                                    .map_err(|e| e.to_string())
                            } else {
                                Err("No window info".to_string())
                            }
                        } else {
                            Err("No focused window".to_string())
                        }
                    }
                    
                    GetCommands::Config => {
                        serde_json::to_string_pretty(&self.config)
                            .map_err(|e| e.to_string())
                    }
                    
                    GetCommands::Version => {
                        Ok(env!("CARGO_PKG_VERSION").to_string())
                    }
                }
            }
            
            Commands::Set { what } => {
                match what {
                    SetCommands::Layout { layout } => {
                        let mut wm = self.window_manager.lock().unwrap();
                        if let Some(layout_config) = self.config.layouts.get(&layout) {
                            wm.set_layout(wm.current_tag, layout_config.clone());
                            Ok(format!("Set layout to {}", layout))
                        } else {
                            Err(format!("Unknown layout: {}", layout))
                        }
                    }
                    
                    SetCommands::Gap { size } => {
                        let mut wm = self.window_manager.lock().unwrap();
                        wm.config.gaps = size;
                        Ok(format!("Set gap size to {}", size))
                    }
                    
                    SetCommands::Border { width } => {
                        let mut wm = self.window_manager.lock().unwrap();
                        wm.config.border_width = width;
                        Ok(format!("Set border width to {}", width))
                    }
                }
            }
            
            Commands::Reload => {
                // TODO: Reload configuration
                Ok("Configuration reloaded".to_string())
            }
            
            Commands::Exit => {
                // TODO: Graceful exit
                Ok("Exiting instantWM".to_string())
            }
        }
    }
}

pub fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    
    // TODO: Connect to running instantWM instance via socket
    // For now, just print the command
    println!("Would execute: {:?}", cli.command);
    
    Ok(())
}