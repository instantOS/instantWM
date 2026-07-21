mod ctl;

use clap::Parser;
use ctl::{Cli, IpcClient, format_response, get_default_socket};
use instantwm::ipc_types::{IpcCommand, Response};
use std::str::FromStr;
use std::time::{Duration, Instant};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctl::commands::ScratchpadAction;
    use clap::Parser;
    use instantwm::ipc_types::{ScratchpadInitialStatus, WindowCommand};

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
                    ..
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
                    ..
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
                    ..
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

    #[test]
    fn scratchpad_show_defaults_name_when_omitted() {
        let cmd: IpcCommand = Cli::parse_from(["instantwmctl", "scratchpad", "show"])
            .command
            .into();

        assert!(matches!(
            cmd,
            IpcCommand::Scratchpad(instantwm::ipc_types::ScratchpadCommand::Show(Some(name)))
                if name == "instantwm_scratchpad"
        ));
    }

    #[test]
    fn scratchpad_hide_defaults_name_when_omitted() {
        let cmd: IpcCommand = Cli::parse_from(["instantwmctl", "scratchpad", "hide"])
            .command
            .into();

        assert!(matches!(
            cmd,
            IpcCommand::Scratchpad(instantwm::ipc_types::ScratchpadCommand::Hide(Some(name)))
                if name == "instantwm_scratchpad"
        ));
    }

    #[test]
    fn parses_window_info_command() {
        let cmd: IpcCommand = Cli::parse_from(["instantwmctl", "window", "info", "42"])
            .command
            .into();

        assert!(matches!(
            cmd,
            IpcCommand::Window(WindowCommand::Info(Some(42)))
        ));
    }

    #[test]
    fn parses_window_resize_command() {
        let cmd: IpcCommand = Cli::parse_from([
            "instantwmctl",
            "window",
            "resize",
            "42",
            "--monitor",
            "1",
            "--x",
            "10",
            "--y",
            "20",
            "--width",
            "800",
            "--height",
            "600",
        ])
        .command
        .into();

        assert!(matches!(
            cmd,
            IpcCommand::Window(WindowCommand::Resize {
                window_id: Some(42),
                monitor: Some(monitor),
                x: 10,
                y: 20,
                width: 800,
                height: 600,
            }) if monitor == "1"
        ));
    }

    #[test]
    fn parses_window_resize_without_monitor() {
        let cmd: IpcCommand = Cli::parse_from([
            "instantwmctl",
            "window",
            "resize",
            "--x",
            "10",
            "--y",
            "20",
            "--width",
            "800",
            "--height",
            "600",
        ])
        .command
        .into();

        assert!(matches!(
            cmd,
            IpcCommand::Window(WindowCommand::Resize {
                window_id: None,
                monitor: None,
                x: 10,
                y: 20,
                width: 800,
                height: 600,
            })
        ));
    }

    #[test]
    fn parses_normalized_test_pointer_move() {
        let cmd: IpcCommand = Cli::parse_from([
            "instantwmctl",
            "test",
            "pointer",
            "move",
            "0.5",
            "0.01",
            "--normalized",
        ])
        .command
        .into();

        assert!(matches!(
            cmd,
            IpcCommand::Test(instantwm::ipc_types::TestCommand::PointerMove {
                x: 0.5,
                y: 0.01,
                normalized: true,
            })
        ));
    }

    #[test]
    fn parses_test_window_tag() {
        let cmd: IpcCommand = Cli::parse_from(["instantwmctl", "test", "window", "tag", "42", "3"])
            .command
            .into();
        assert!(matches!(
            cmd,
            IpcCommand::Test(instantwm::ipc_types::TestCommand::TagWindow {
                window_id: 42,
                tag: 3,
            })
        ));
    }

    #[test]
    fn config_list_accepts_prefix_arg() {
        let cli = Cli::parse_from(["instantwmctl", "config", "list", "fonts"]);
        match cli.command {
            ctl::CommandKind::Config {
                action: crate::ctl::commands::ConfigAction::List { prefix },
            } => assert_eq!(prefix.as_deref(), Some("fonts")),
            other => panic!("expected Config List, got {other:?}"),
        }
    }

    #[test]
    fn config_list_prefix_is_optional() {
        let cli = Cli::parse_from(["instantwmctl", "config", "list"]);
        match cli.command {
            ctl::CommandKind::Config {
                action: crate::ctl::commands::ConfigAction::List { prefix },
            } => assert!(prefix.is_none()),
            other => panic!("expected Config List, got {other:?}"),
        }
    }

    #[test]
    fn config_list_maps_to_unfiltered_ipc_list() {
        // The prefix is client-side only; the WM always receives a bare List.
        let cmd: IpcCommand = Cli::parse_from(["instantwmctl", "config", "list", "fonts"])
            .command
            .into();
        assert!(matches!(
            cmd,
            IpcCommand::Config(instantwm::ipc_types::ConfigCommand::List)
        ));
    }

    #[test]
    fn validate_list_prefix_accepts_known_sections() {
        assert!(validate_list_prefix("fonts").is_ok());
        assert!(validate_list_prefix("fonts.fonts").is_ok());
        assert!(validate_list_prefix("input").is_ok());
        // Map-section ids contain dots/colons; the section is still `input`.
        assert!(validate_list_prefix("input.type:touchpad").is_ok());
    }

    #[test]
    fn validate_list_prefix_rejects_unknown_section() {
        assert!(validate_list_prefix("frobnicate").is_err());
        assert!(validate_list_prefix("frobnicate.field").is_err());
    }

    #[test]
    fn validate_list_prefix_explains_display_is_hidden() {
        let err = validate_list_prefix("display.width").unwrap_err();
        assert!(err.contains("derived"), "got: {err}");
    }

    #[test]
    fn filter_config_list_narrows_to_section() {
        let entries = vec![
            ("fonts.config_font".to_string(), "x".to_string()),
            ("fonts.fonts".to_string(), "y".to_string()),
            ("layout.inner_gap".to_string(), "0".to_string()),
        ];
        match filter_config_list(Response::ConfigList(entries), "fonts") {
            Response::ConfigList(rows) => {
                assert_eq!(rows.len(), 2);
                assert!(rows.iter().all(|(k, _)| k.starts_with("fonts.")));
            }
            other => panic!("expected ConfigList, got {other:?}"),
        }
    }

    #[test]
    fn filter_config_list_matches_single_leaf() {
        let entries = vec![
            ("fonts.config_font".to_string(), "x".to_string()),
            ("fonts.fonts".to_string(), "y".to_string()),
        ];
        match filter_config_list(Response::ConfigList(entries), "fonts.fonts") {
            Response::ConfigList(rows) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].0, "fonts.fonts");
            }
            other => panic!("expected ConfigList, got {other:?}"),
        }
    }

    #[test]
    fn filter_config_list_does_not_match_sibling_prefix() {
        // `fonts` must not match a `fontsx.*` sibling (trailing-dot check).
        let entries = vec![
            ("fonts.fonts".to_string(), "y".to_string()),
            ("fontsx.thing".to_string(), "z".to_string()),
        ];
        match filter_config_list(Response::ConfigList(entries), "fonts") {
            Response::ConfigList(rows) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].0, "fonts.fonts");
            }
            other => panic!("expected ConfigList, got {other:?}"),
        }
    }
}

fn main() {
    // instantwmctl produces piped output (`--json` feeds jq/python). Rust
    // ignores SIGPIPE by default, so a closed downstream pipe surfaces as a
    // BrokenPipe write error that `println!` turns into a panic + backtrace.
    // Restore the default disposition so a closed reader terminates us
    // silently (exit 141), matching standard Unix CLI behavior.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();

    // `config list <prefix>` narrows the full list client-side after the IPC
    // round-trip, so capture the prefix here and apply it to the response.
    let config_list_prefix = match &cli.command {
        ctl::CommandKind::Config {
            action: ctl::commands::ConfigAction::List { prefix },
        } => prefix.clone(),
        _ => None,
    };

    // Validate the prefix up front so a bad section errors before we bother the
    // running WM.
    if let Some(prefix) = &config_list_prefix
        && let Err(msg) = validate_list_prefix(prefix)
    {
        eprintln!("instantwmctl: {msg}");
        std::process::exit(1);
    }

    if handle_local_test_command(&cli) {
        return;
    }

    let command = match &cli.command {
        ctl::CommandKind::Layout { name } if name.as_deref() == Some("list") => {
            print_layout_list(cli.json);
            return;
        }
        ctl::CommandKind::Layout { name } => {
            let Some(name) = name else {
                eprintln!(
                    "instantwmctl: layout name required (use 'instantwmctl layout list' to see layouts)"
                );
                std::process::exit(1);
            };
            let Ok(layout) = instantwm::layouts::LayoutCommand::from_str(name) else {
                eprintln!(
                    "instantwmctl: invalid layout '{name}' (use 'instantwmctl layout list' to see layouts)"
                );
                std::process::exit(1);
            };
            IpcCommand::Layout(layout)
        }
        ctl::CommandKind::Config { action } => match action {
            ctl::commands::ConfigAction::Default => {
                println!(
                    "{}",
                    instantwm::config::config_toml::generate_commented_config()
                );
                return;
            }
            _ => cli.command.clone().into(),
        },
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
        _ => cli.command.clone().into(),
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

    let response = match config_list_prefix {
        Some(prefix) => filter_config_list(response, &prefix),
        None => response,
    };

    format_response(&response, cli.json);
}

/// Execute test operations that require multiple IPC round trips without
/// blocking the compositor's event loop.
fn handle_local_test_command(cli: &Cli) -> bool {
    use ctl::commands::{TestAction, TestPointerAction, TestWaitAction};

    let ctl::CommandKind::Test { action } = &cli.command else {
        return false;
    };

    match action {
        TestAction::Pointer {
            action:
                TestPointerAction::Path {
                    points,
                    duration_ms,
                    hz,
                    normalized,
                },
        } => {
            if let Err(message) = run_pointer_path(
                points,
                *duration_ms,
                *hz,
                *normalized,
                cli.ignore_version_mismatches,
                cli.json,
            ) {
                exit_with_error(&message);
            }
            true
        }
        TestAction::Wait {
            action:
                TestWaitAction::Windows {
                    count,
                    timeout_ms,
                    poll_ms,
                    exact,
                },
        } => {
            if let Err(message) = wait_for_windows(
                *count,
                *timeout_ms,
                *poll_ms,
                *exact,
                cli.ignore_version_mismatches,
                cli.json,
            ) {
                exit_with_error(&message);
            }
            true
        }
        _ => false,
    }
}

fn parse_path_point(raw: &str) -> Result<(f64, f64), String> {
    let Some((x, y)) = raw.split_once(',') else {
        return Err(format!("invalid point '{raw}'; expected X,Y"));
    };
    let x = x
        .parse::<f64>()
        .map_err(|_| format!("invalid x coordinate in '{raw}'"))?;
    let y = y
        .parse::<f64>()
        .map_err(|_| format!("invalid y coordinate in '{raw}'"))?;
    if !x.is_finite() || !y.is_finite() {
        return Err(format!("coordinates in '{raw}' must be finite"));
    }
    Ok((x, y))
}

fn run_pointer_path(
    raw_points: &[String],
    duration_ms: u64,
    hz: u32,
    normalized: bool,
    ignore_version: bool,
    json: bool,
) -> Result<(), String> {
    if duration_ms == 0 {
        return Err("pointer path duration must be greater than zero".to_string());
    }
    if !(1..=240).contains(&hz) {
        return Err("pointer path frequency must be between 1 and 240 Hz".to_string());
    }
    let points = raw_points
        .iter()
        .map(|point| parse_path_point(point))
        .collect::<Result<Vec<_>, _>>()?;

    let intervals = ((u128::from(duration_ms) * u128::from(hz)) / 1000).max(1) as u64;
    let started = Instant::now();
    for sample in 0..=intervals {
        let path_position = sample as f64 / intervals as f64 * (points.len() - 1) as f64;
        let segment = (path_position.floor() as usize).min(points.len() - 2);
        let fraction = (path_position - segment as f64).min(1.0);
        let (x0, y0) = points[segment];
        let (x1, y1) = points[segment + 1];
        let x = x0 + (x1 - x0) * fraction;
        let y = y0 + (y1 - y0) * fraction;
        send_once(
            IpcCommand::Test(instantwm::ipc_types::TestCommand::PointerMove { x, y, normalized }),
            ignore_version,
        )?;

        if sample < intervals {
            let target = started
                + Duration::from_nanos(
                    (u128::from(duration_ms) * 1_000_000 * u128::from(sample + 1)
                        / u128::from(intervals)) as u64,
                );
            if let Some(remaining) = target.checked_duration_since(Instant::now()) {
                std::thread::sleep(remaining);
            }
        }
    }

    if json {
        println!(
            "{}",
            serde_json::json!({
                "operation": "pointer-path",
                "samples": intervals + 1,
                "duration_ms": duration_ms,
                "frequency_hz": hz,
                "normalized": normalized,
            })
        );
    } else {
        println!("pointer path complete: {} samples", intervals + 1);
    }
    Ok(())
}

fn wait_for_windows(
    expected: usize,
    timeout_ms: u64,
    poll_ms: u64,
    exact: bool,
    ignore_version: bool,
    json: bool,
) -> Result<(), String> {
    if timeout_ms == 0 || poll_ms == 0 {
        return Err("wait timeout and polling interval must be greater than zero".to_string());
    }
    let started = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    loop {
        let response = send_once(
            IpcCommand::Window(instantwm::ipc_types::WindowCommand::List(None)),
            ignore_version,
        )?;
        let Response::WindowList(windows) = response else {
            return Err("unexpected response while waiting for windows".to_string());
        };
        let actual = windows.len();
        if (exact && actual == expected) || (!exact && actual >= expected) {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "condition": "window-count",
                        "expected": expected,
                        "actual": actual,
                        "exact": exact,
                        "elapsed_ms": started.elapsed().as_millis(),
                    })
                );
            } else {
                println!("window wait complete: {actual} mapped");
            }
            return Ok(());
        }
        if started.elapsed() >= timeout {
            return Err(format!(
                "timed out after {timeout_ms}ms waiting for {} {expected} windows (saw {actual})",
                if exact { "exactly" } else { "at least" }
            ));
        }
        std::thread::sleep(Duration::from_millis(poll_ms));
    }
}

fn send_once(command: IpcCommand, ignore_version: bool) -> Result<Response, String> {
    let socket = get_default_socket();
    let mut client = IpcClient::connect(&socket)
        .map_err(|error| format!("connect failed ({socket}): {error}"))?;
    match client
        .send(command, ignore_version)
        .map_err(|error| error.to_string())?
    {
        Response::Err(message) => Err(message),
        response => Ok(response),
    }
}

fn exit_with_error(message: &str) -> ! {
    eprintln!("instantwmctl: {message}");
    std::process::exit(1);
}

/// Reject prefixes whose top-level section isn't listable, with a helpful
/// message. Returns `Ok(())` for any prefix under a known section (including
/// unknown sub-fields — those simply produce an empty list downstream).
///
/// The set of valid sections comes from the WM (`instantwm::ipc::config`) so
/// this stays a single source of truth rather than a parallel list.
fn validate_list_prefix(prefix: &str) -> Result<(), String> {
    use instantwm::ipc::config::{RuntimeConfigSection, SectionStatus, section_status};
    let section = prefix.split('.').next().unwrap_or(prefix);
    match section_status(section) {
        SectionStatus::Exposed => Ok(()),
        SectionStatus::Hidden => Err(format!(
            "'{section}' is derived from outputs and not exposed at runtime"
        )),
        SectionStatus::Unknown => Err(format!(
            "unknown section '{section}' (known: {})",
            RuntimeConfigSection::EXPOSED
                .into_iter()
                .map(RuntimeConfigSection::name)
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Narrow a `ConfigList` to keys under `prefix`.
///
/// A key matches when it equals `prefix` (a leaf key, e.g. `fonts.fonts`) or
/// sits beneath it (a section or `section.id`, e.g. `fonts` or
/// `input.type:touchpad`). The trailing-dot check prevents `fonts` from
/// matching a sibling like `fontsx`.
fn filter_config_list(response: Response, prefix: &str) -> Response {
    let dot = format!("{prefix}.");
    match response {
        Response::ConfigList(entries) => {
            let filtered: Vec<_> = entries
                .into_iter()
                .filter(|(k, _)| k == prefix || k.starts_with(&dot))
                .collect();
            Response::ConfigList(filtered)
        }
        other => other,
    }
}

fn print_layout_list(json: bool) {
    let layouts: Vec<_> = instantwm::layouts::LayoutCommand::all()
        .iter()
        .map(|layout| {
            serde_json::json!({
                "name": layout.name(),
                "label": layout.label(),
                "description": layout.description(),
                "symbol": layout.symbol(),
                "tiling": layout.results_in_tiling(),
            })
        })
        .collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&layouts).unwrap());
    } else {
        for layout in instantwm::layouts::LayoutCommand::all() {
            println!(
                "{:<13} {:<14} {}",
                layout.name(),
                layout.symbol(),
                layout.description()
            );
        }
    }
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
            if let Ok(data) = bincode::encode_to_vec(&request, bincode::config::standard()) {
                let _ = stream.write_all(&data);
            }
        }
        line.clear();
    }
    std::process::exit(0);
}
