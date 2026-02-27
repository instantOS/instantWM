use clap::{Parser, Subcommand};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

#[derive(Debug, Parser)]
#[command(name = "instantwmctl", version, disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Debug, Subcommand)]
enum CommandKind {
    List,
    Geom { window_id: Option<u32> },
    Spawn { command: Vec<String> },
    Close { window_id: Option<u32> },
}

fn main() {
    let cli = Cli::parse();
    let request = match cli.command {
        CommandKind::List => "list\n".to_string(),
        CommandKind::Geom { window_id } => match window_id {
            Some(id) => format!("geom {}\n", id),
            None => "geom\n".to_string(),
        },
        CommandKind::Spawn { command } => {
            if command.is_empty() {
                eprintln!("instantwmctl: spawn requires a command");
                std::process::exit(2);
            }
            format!("spawn {}\n", command.join(" "))
        }
        CommandKind::Close { window_id } => match window_id {
            Some(id) => format!("close {}\n", id),
            None => "close\n".to_string(),
        },
    };

    let socket = std::env::var("INSTANTWM_SOCKET")
        .unwrap_or_else(|_| format!("/tmp/instantwm-{}.sock", unsafe { libc::geteuid() }));
    let mut stream = match UnixStream::connect(&socket) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("instantwmctl: connect failed ({}): {}", socket, err);
            std::process::exit(1);
        }
    };
    if let Err(err) = stream.write_all(request.as_bytes()) {
        eprintln!("instantwmctl: write failed: {}", err);
        std::process::exit(1);
    }
    let _ = stream.shutdown(std::net::Shutdown::Write);

    let mut response = String::new();
    if let Err(err) = stream.read_to_string(&mut response) {
        eprintln!("instantwmctl: read failed: {}", err);
        std::process::exit(1);
    }
    print!("{}", response);
    if response.starts_with("ERR") {
        std::process::exit(1);
    }
}
