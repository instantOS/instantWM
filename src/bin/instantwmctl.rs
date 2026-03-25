mod ctl;

use clap::Parser;
use ctl::{Cli, IpcClient, command_to_ipc, format_response, get_default_socket};
use instantwm::ipc_types::IpcCommand;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctl::commands::ScratchpadAction;
    use clap::Parser;
    use instantwm::ipc_types::ScratchpadInitialStatus;

    #[test]
    fn parses_reload_command() {
        let cli = Cli::parse_from(["instantwmctl", "reload"]);
        assert!(matches!(cli.command, ctl::CommandKind::Reload));
    }

    #[test]
    fn parses_scratchpad_create_status_flag() {
        let cli = Cli::parse_from([
            "instantwmctl",
            "scratchpad",
            "create",
            "term",
            "--status",
            "shown",
        ]);

        assert!(matches!(
            cli.command,
            ctl::CommandKind::Scratchpad {
                action: ScratchpadAction::Create {
                    name,
                    window_id: None,
                    status: ScratchpadInitialStatus::Shown,
                }
            } if name == "term"
        ));
    }

    #[test]
    fn scratchpad_create_defaults_to_hidden() {
        let cli = Cli::parse_from(["instantwmctl", "scratchpad", "create", "term"]);

        assert!(matches!(
            cli.command,
            ctl::CommandKind::Scratchpad {
                action: ScratchpadAction::Create {
                    name,
                    window_id: None,
                    status: ScratchpadInitialStatus::Hidden,
                }
            } if name == "term"
        ));
    }

    #[test]
    fn scratchpad_create_defaults_name_when_omitted() {
        let cli = Cli::parse_from(["instantwmctl", "scratchpad", "create"]);

        assert!(matches!(
            cli.command,
            ctl::CommandKind::Scratchpad {
                action: ScratchpadAction::Create {
                    name,
                    window_id: None,
                    status: ScratchpadInitialStatus::Hidden,
                }
            } if name == "instantwm_scratchpad"
        ));
    }

    #[test]
    fn parses_scratchpad_hide_all_flag() {
        let cli = Cli::parse_from(["instantwmctl", "scratchpad", "hide", "--all"]);

        assert!(matches!(
            cli.command,
            ctl::CommandKind::Scratchpad {
                action: ScratchpadAction::Hide {
                    name: None,
                    all: true
                }
            }
        ));
    }
}

fn main() {
    let cli = Cli::parse();

    let command = match &cli.command {
        ctl::CommandKind::Action { name, args, list } => {
            if *list {
                let actions = instantwm::config::keybind_config::get_actions_for_ipc();
                let response = instantwm::ipc_types::Response::ActionList(actions);
                format_response(&response, cli.json);
                return;
            }
            let name = match name {
                Some(name) => name.clone(),
                None => {
                    eprintln!(
                        "instantwmctl: action name required (use --list to see available actions)"
                    );
                    std::process::exit(1);
                }
            };
            IpcCommand::RunAction {
                name,
                args: args.clone(),
            }
        }
        _ => command_to_ipc(cli.command.clone()),
    };

    if let IpcCommand::UpdateStatus(text) = &command
        && text == "-"
    {
        handle_status_from_stdin(cli.ignore_version_mismatches);
        return;
    }

    let socket = get_default_socket();
    let mut client = match IpcClient::connect(&socket) {
        Ok(c) => c,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                eprintln!(
                    "instantwmctl: instantWM is not running (socket not found: {})",
                    socket
                );
                eprintln!("Make sure instantWM is started before using instantwmctl.");
            } else {
                eprintln!("instantwmctl: connect failed ({}): {}", socket, err);
            }
            std::process::exit(1);
        }
    };

    let response = match client.send(command, cli.ignore_version_mismatches) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("instantwmctl: {}", e);
            std::process::exit(1);
        }
    };

    format_response(&response, cli.json);
}

fn handle_status_from_stdin(ignore_version_mismatches: bool) {
    use std::io::{BufRead, Write};

    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut line = String::new();

    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        let trim_line = line.trim();
        if trim_line == "[" || trim_line.starts_with("{\"version\"") || trim_line.is_empty() {
            line.clear();
            continue;
        }

        let socket = get_default_socket();

        if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&socket) {
            let cmd = IpcCommand::UpdateStatus(trim_line.to_string());
            let request = if ignore_version_mismatches {
                instantwm::ipc_types::IpcRequest::new_ignore_version(cmd, true)
            } else {
                instantwm::ipc_types::IpcRequest::new(cmd)
            };
            if let Ok(data) = serde_json::to_vec(&request) {
                let _ = stream.write_all(&data);
            }
        }
        line.clear();
    }
    std::process::exit(0);
}
