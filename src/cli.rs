use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::process::Command as ProcessCommand;

#[derive(Parser)]
#[command(name = "instantctl")]
#[command(about = "Control instantWM from the command line")]
#[command(version)]
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

    /// Reload configuration
    Reload,

    /// Exit instantWM
    Exit,
}

#[derive(Subcommand, Debug)]
pub enum GetCommands {
    /// Get current tag
    Tag,
    /// Get list of windows on current tag
    Windows,
    /// Get focused window information
    Focused,
    /// Get version information
    Version,
}

#[derive(Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub class: String,
    pub floating: bool,
    pub tag: usize,
}

pub fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Spawn { command, args } => {
            match ProcessCommand::new(&command).args(&args).spawn() {
                Ok(_) => println!("Spawned: {} {}", command, args.join(" ")),
                Err(e) => eprintln!("Failed to spawn {}: {}", command, e),
            }
        }
        Commands::Get { what } => match what {
            GetCommands::Version => {
                println!("{}", env!("CARGO_PKG_VERSION"));
            }
            _ => {
                eprintln!("IPC not implemented yet. Run instantWM in the main terminal.");
            }
        },
        _ => {
            eprintln!("IPC not implemented yet. Please use keybindings to control the compositor.");
            eprintln!("Run 'instantwm' to start the compositor.");
        }
    }

    Ok(())
}
