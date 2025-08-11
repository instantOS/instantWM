pub mod cli;
pub mod compositor;
pub mod config;
pub mod error;
pub mod input;
pub mod top_bar;
pub mod types;
pub mod window_manager;

pub use error::{InstantError as Error, Result};
pub use types::Config;
pub use types::*;

// Re-export commonly used smithay types for convenience
pub use smithay::{
    desktop::Window,
    reexports::wayland_server::{Display, DisplayHandle},
    utils::{Point, Rectangle, Size},
};

use tracing::info;
use tracing_subscriber::Layer;

/// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const NAME: &str = "InstantWM";

/// Initialize the logging system
pub fn init_logging() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(
                    tracing_subscriber::EnvFilter::from_default_env()
                        .add_directive("instantwm_rs=debug".parse().unwrap())
                        .add_directive("smithay=info".parse().unwrap()),
                ),
        )
        .init();

    info!("InstantWM v{} - Wayland compositor", VERSION);
}

/// Print version information
pub fn print_version() {
    println!("{} {}", NAME, VERSION);
    println!("A fast, lightweight Wayland compositor inspired by instantWM");
    println!("Built with Smithay 0.7.0");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_loading() {
        let config = Config::default();
        assert!(!config.tags.names.is_empty());
        assert!(!config.keybindings.is_empty());
    }

    #[test]
    fn test_config_validation() {
        let config = Config::default();
        let errors = config.validate();
        assert!(errors.is_empty(), "Default config should be valid");
    }

    #[test]
    fn test_window_rules() {
        let config = Config::default();
        let rules = config.get_rules_for_window("Pavucontrol", "Volume Control");
        assert!(!rules.is_empty(), "Should have rules for Pavucontrol");
        assert!(rules[0].floating, "Pavucontrol should be floating");
    }
}
