use clap::{Parser, ValueEnum};
mod autostart;
mod common_wayland;
mod drm;
mod locale;
mod wayland;
mod x11;

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

    if cli.print_config {
        let config = crate::config::config_toml::ThemeConfig::default();
        let toml = toml::to_string_pretty(&config).expect("failed to serialize default config");
        println!("{toml}");
        return;
    }

    if cli.list_actions {
        let actions = crate::config::keybind_config::get_named_actions();
        for action in actions {
            println!("{action}");
        }
        return;
    }

    if locale::set_locale().is_err() {
        eprintln!("warning: no locale support");
    }

    match cli.backend {
        CliBackend::X11 => x11::run(),
        CliBackend::Nested => wayland::run(),
        CliBackend::Drm => drm::run(),
    }
}
