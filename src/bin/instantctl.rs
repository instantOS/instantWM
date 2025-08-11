use clap::Parser;
use instantwm_rs::cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();

    // TODO: Implement actual socket communication with running instantWM
    // For now, just print what would be executed

    match cli.command {
        Commands::Tag { number } => {
            println!("Switching to tag {}", number);
        }
        Commands::MoveToTag { number } => {
            println!("Moving focused window to tag {}", number);
        }
        Commands::ToggleFloating => {
            println!("Toggling floating mode for focused window");
        }
        Commands::Close => {
            println!("Closing focused window");
        }
        Commands::Spawn { command, args } => {
            println!("Spawning: {} {}", command, args.join(" "));
        }
        Commands::Get { what } => {
            println!("Getting: {:?}", what);
        }
        Commands::Reload => {
            println!("Reloading configuration");
        }
        Commands::Exit => {
            println!("Exiting instantWM");
        }
    }
}
