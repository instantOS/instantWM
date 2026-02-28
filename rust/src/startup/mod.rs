use clap::{Parser, ValueEnum};
use std::process::exit;

use crate::xresources::list_xresources;

mod autostart;
mod locale;
mod wayland;
mod x11;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliBackend {
    X11,
    Wayland,
}

#[derive(Debug, Parser)]
#[command(name = "instantwm", version, disable_help_subcommand = true)]
struct Cli {
    #[arg(short = 'X', long = "xresources")]
    xresources: bool,

    #[arg(long, value_enum, default_value_t = CliBackend::X11)]
    backend: CliBackend,
}

pub fn run() {
    let cli = Cli::parse();

    if cli.xresources {
        list_xresources();
        exit(0);
    }

    if locale::set_locale().is_err() {
        eprintln!("warning: no locale support");
    }

    match cli.backend {
        CliBackend::X11 => x11::run(),
        CliBackend::Wayland => wayland::run(),
    }
}
