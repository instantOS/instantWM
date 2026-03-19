use clap::{Parser, ValueEnum};
use std::env;
pub mod autostart;
mod locale;
pub(crate) mod x11;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliBackend {
    X11,
    /// Run as a nested Wayland compositor inside an existing Wayland or X11 session.
    Nested,
    /// Run as a standalone Wayland compositor directly on DRM/KMS hardware.
    Drm,
}

#[derive(Debug, Parser)]
#[command(name = "instantwm", version, disable_help_subcommand = true)]
struct Cli {
    /// Print an example config.toml and exit.
    #[arg(long = "print-config")]
    print_config: bool,
    /// Print all valid named actions for keybinds and exit.
    #[arg(long = "list-actions")]
    list_actions: bool,
    #[arg(long, value_enum, default_value_t = CliBackend::X11)]
    backend: CliBackend,
}

pub fn run() {
    let cli = Cli::parse();

    // Set environment variables to identify instantWM
    unsafe { env::set_var("INSTANTWM", "1") };
    match cli.backend {
        CliBackend::X11 => unsafe { env::set_var("INSTANTWM_BACKEND", "x11") },
        CliBackend::Nested => unsafe { env::set_var("INSTANTWM_BACKEND", "wayland-nested") },
        CliBackend::Drm => unsafe { env::set_var("INSTANTWM_BACKEND", "wayland-drm") },
    }

    if cli.print_config {
        let config = crate::config::config_toml::ThemeConfig::default();
        let toml = toml::to_string_pretty(&config).expect("failed to serialize default config");
        println!("{toml}");
        return;
    }

    if cli.list_actions {
        use crate::config::keybind_config::print_actions;
        print_actions(false);
        std::process::exit(0);
    }

    if locale::set_locale().is_err() {
        eprintln!("warning: no locale support");
    }

    match cli.backend {
        CliBackend::X11 => x11::run(),
        CliBackend::Nested => crate::wayland::runtime::winit::run(),
        CliBackend::Drm => crate::wayland::runtime::drm::run(),
    }
}
