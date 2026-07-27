use clap::{Parser, ValueEnum};
use std::env;
pub mod autostart;
mod default_commands;

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
    /// Window-system backend to use.
    ///
    /// When omitted, the backend is auto-detected from the environment:
    /// - A running Wayland session (`WAYLAND_DISPLAY` set) selects the nested
    ///   Wayland backend, so instantWM runs as a nested compositor instead of
    ///   hijacking the host's Xwayland server.
    /// - A running X11 session (`DISPLAY` set, no `WAYLAND_DISPLAY`) selects
    ///   the X11 backend.
    /// - Neither set (e.g. a bare tty) selects the DRM/KMS backend so
    ///   instantWM takes over the hardware directly.
    #[arg(long, value_enum)]
    backend: Option<CliBackend>,
}

/// Resolve the backend to use, applying auto-detection when the user did not
/// pass `--backend` explicitly.
fn resolve_backend(cli: &Cli) -> CliBackend {
    if let Some(explicit) = cli.backend {
        return explicit;
    }

    let wayland = env::var_os("WAYLAND_DISPLAY").is_some();
    let x11 = env::var_os("DISPLAY").is_some();
    match (wayland, x11) {
        (true, _) => {
            eprintln!(
                "instantwm: Wayland session detected (WAYLAND_DISPLAY is set); \
                 defaulting to the nested Wayland backend. \
                 Pass --backend x11 to force the X11 backend."
            );
            CliBackend::Nested
        }
        (false, true) => CliBackend::X11,
        (false, false) => {
            eprintln!(
                "instantwm: no display session detected; \
                 defaulting to the DRM/KMS backend. \
                 Pass --backend x11 or --backend nested to override."
            );
            CliBackend::Drm
        }
    }
}

pub fn run() {
    let cli = Cli::parse();

    let backend = resolve_backend(&cli);

    // Set environment variables to identify instantWM
    unsafe { env::set_var("INSTANTWM", "1") };
    match backend {
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

    default_commands::ensure_default_command_aliases();

    match backend {
        CliBackend::X11 => crate::backend::x11::startup::run(),
        CliBackend::Nested => crate::wayland::runtime::winit::run(),
        CliBackend::Drm => crate::wayland::runtime::drm::run(),
    }
}
