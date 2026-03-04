use clap::{Parser, ValueEnum};
mod autostart;
mod drm;
mod locale;
mod wayland;
mod x11;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliBackend {
    X11,
    Wayland,
    /// Run as a standalone Wayland compositor directly on DRM/KMS hardware.
    Drm,
}

#[derive(Debug, Parser)]
#[command(name = "instantwm", version, disable_help_subcommand = true)]
struct Cli {
    /// Print an example config.toml and exit.
    #[arg(long = "print-config")]
    print_config: bool,
    #[arg(long, value_enum, default_value_t = CliBackend::X11)]
    backend: CliBackend,
}

pub fn run() {
    let cli = Cli::parse();

    if cli.print_config {
        println!("{}", crate::config::config_doc::CONFIG_DOC);
        return;
    }

    if locale::set_locale().is_err() {
        eprintln!("warning: no locale support");
    }

    match cli.backend {
        CliBackend::X11 => x11::run(),
        CliBackend::Wayland => wayland::run(),
        CliBackend::Drm => drm::run(),
    }
}
