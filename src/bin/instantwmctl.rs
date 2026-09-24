mod ctl;

use clap::Parser;
use ctl::commands::{ConfigAction, TestAction};
use ctl::format::print_json;
use ctl::{Cli, CommandKind, format_response};
use instantwm::ipc_types::{IpcCommand, Response, TestCommand, WindowCommand};
use std::time::{Duration, Instant};

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
    let (json, ignore_version) = (cli.json, cli.ignore_version_mismatches);

    let command = match cli.command.into_ipc() {
        Ok(IpcCommand::UpdateStatus(text)) if text == "-" => {
            return status_from_stdin(ignore_version);
        }
        Ok(command) => command,
        Err(local) => {
            if let Err(message) = run_local(local, json, ignore_version) {
                exit_with_error(&message);
            }
            return;
        }
    };

    match ctl::ipc::send(command, ignore_version) {
        Ok(response) => format_response(&response, json),
        Err(error) => exit_with_error(&error.to_string()),
    }
}

/// Commands answered by the client itself, possibly through several IPC round
/// trips (which must not block the compositor's event loop).
fn run_local(command: CommandKind, json: bool, ignore_version: bool) -> Result<(), String> {
    match command {
        CommandKind::Action { .. } => {
            let actions = instantwm::actions::action_infos();
            if json {
                print_json(&actions);
            } else {
                print!("{}", instantwm::actions::format_action_list(&actions));
            }
            Ok(())
        }
        CommandKind::Config {
            action: ConfigAction::Default,
        } => {
            println!(
                "{}",
                instantwm::config::config_toml::generate_commented_config()
            );
            Ok(())
        }
        CommandKind::Test {
            action:
                TestAction::PointerPath {
                    points,
                    duration_ms,
                    hz,
                    normalized,
                },
        } => run_pointer_path(&points, duration_ms, hz, normalized, ignore_version, json),
        CommandKind::Test {
            action:
                TestAction::WaitWindows {
                    count,
                    timeout_ms,
                    poll_ms,
                    exact,
                },
        } => wait_for_windows(count, timeout_ms, poll_ms, exact, ignore_version, json),
        other => unreachable!("{other:?} is sent to the compositor"),
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
            IpcCommand::Test(TestCommand::PointerMove { x, y, normalized }),
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
            IpcCommand::Window(WindowCommand::List { window_id: None }),
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
    match ctl::ipc::send(command, ignore_version).map_err(|error| error.to_string())? {
        Response::Err(message) => Err(message),
        response => Ok(response),
    }
}

fn exit_with_error(message: &str) -> ! {
    eprintln!("instantwmctl: {message}");
    std::process::exit(1);
}

/// Forward each stdin line as a status update (i3bar JSON headers skipped).
fn status_from_stdin(ignore_version: bool) {
    use std::io::BufRead;

    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let line = line.trim();
        if line.is_empty() || line == "[" || line.starts_with("{\"version\"") {
            continue;
        }
        let _ = ctl::ipc::send(IpcCommand::UpdateStatus(line.to_string()), ignore_version);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use instantwm::ipc_types::{MonitorCommand, ScratchpadCommand, Transform};

    fn ipc(argv: &[&str]) -> IpcCommand {
        let cli = Cli::parse_from(std::iter::once("instantwmctl").chain(argv.iter().copied()));
        cli.command
            .into_ipc()
            .unwrap_or_else(|local| panic!("{local:?} is handled locally"))
    }

    #[test]
    fn convenience_commands_compile_to_canonical_actions() {
        for (argv, name, args) in [
            (&["toggle", "alt-tag", "on"][..], "toggle_alt_tag", &["on"][..]),
            (&["toggle", "animated"], "toggle_animated", &[]),
            (&["layout", "set", "grid"], "set_layout", &["grid"]),
            (&["mode", "set", "resize"], "set_mode", &["resize"]),
            (&["tag", "view", "4"], "view_tag", &["4"]),
            (&["follow-mon", "prev"], "follow_mon", &["prev"]),
            (&["spawn", "printf", "hello world"], "spawn", &["printf", "hello world"]),
        ] {
            match ipc(argv) {
                IpcCommand::RunAction {
                    name: actual_name,
                    args: actual_args,
                } => {
                    assert_eq!(actual_name, name);
                    assert_eq!(actual_args, args);
                    let parsed = instantwm::actions::NamedAction::parse(&actual_name, &actual_args);
                    assert!(parsed.is_ok(), "{argv:?}: {parsed:?}");
                }
                other => panic!("{argv:?}: expected RunAction, got {other:?}"),
            }
        }
    }

    #[test]
    fn monitor_set_flattens_the_typed_monitor_config() {
        let IpcCommand::Monitor(MonitorCommand::Set { identifier, config }) = ipc(&[
            "monitor",
            "set",
            "DP-2",
            "--transform",
            "90",
            "--enable",
            "false",
            "--mirror",
            "none",
        ]) else {
            panic!("expected monitor set");
        };
        assert_eq!(identifier, "DP-2");
        assert_eq!(config.transform, Some(Transform::Rotate90));
        assert_eq!(config.enable, Some(false));
        assert_eq!(config.mirror.as_deref(), Some("none"));
        assert_eq!(config.scale, None);
    }

    #[test]
    fn scratchpad_commands_default_to_the_shared_scratchpad_name() {
        assert!(matches!(
            ipc(&["scratchpad", "show"]),
            IpcCommand::Scratchpad(ScratchpadCommand::Show { name, all: false })
                if name == instantwm::ipc_types::DEFAULT_SCRATCHPAD_NAME
        ));
        assert!(Cli::try_parse_from(["instantwmctl", "scratchpad", "hide", "term", "--all"]).is_err());
    }

    #[test]
    fn client_side_commands_are_not_sent() {
        for argv in [
            &["action", "--list"][..],
            &["config", "default"],
            &["test", "pointer-path", "0,0", "1,1"],
            &["test", "wait-windows", "2"],
        ] {
            let cli = Cli::parse_from(std::iter::once("instantwmctl").chain(argv.iter().copied()));
            assert!(cli.command.into_ipc().is_err(), "{argv:?}");
        }
    }
}
