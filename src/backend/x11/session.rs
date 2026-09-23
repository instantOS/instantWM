//! X11 session environment for D-Bus activation and systemd user services.
//!
//! The Wayland backend publishes `WAYLAND_DISPLAY` and
//! `XDG_SESSION_TYPE=wayland` to the systemd user manager, which outlives the
//! graphical session. Without this module, the next X11 login inherits those
//! values: D-Bus activated applications (terminals, portals) and services on
//! `graphical-session.target` then believe they run under Wayland and try to
//! reach a compositor that no longer exists.

use std::env;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;

/// Variables describing the X11 session to activated services.
const SESSION_VARS: &[&str] = &[
    "DISPLAY",
    "XAUTHORITY",
    "XDG_SESSION_TYPE",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_DESKTOP",
    "DESKTOP_SESSION",
];

/// Variables that only make sense inside a Wayland session.
const WAYLAND_ONLY_VARS: &[&str] = &["WAYLAND_DISPLAY", "WAYLAND_SOCKET"];

/// Publish the X11 session to the systemd user manager and D-Bus activation
/// environment, replacing leftovers from an earlier Wayland session.
///
/// Must run before `ins autostart` and before anything is started through
/// D-Bus or `graphical-session.target`, so they see the correct environment.
pub fn export_session_env() {
    if host_wayland_session_alive() {
        // The X11 backend is running nested on a live Wayland desktop (e.g.
        // on Xwayland for testing). That desktop owns the session environment.
        log::info!("live Wayland session detected; not exporting the X11 session environment");
        return;
    }

    unsafe {
        env::set_var("XDG_SESSION_TYPE", "x11");
        env::set_var("XDG_CURRENT_DESKTOP", "instantwm");
        env::set_var("XDG_SESSION_DESKTOP", "instantwm");
        env::set_var("DESKTOP_SESSION", "instantwm");
        for var in WAYLAND_ONLY_VARS {
            env::remove_var(var);
        }
    }

    // `dbus-update-activation-environment` can only set variables, so stale
    // Wayland variables have to be removed from systemd explicitly.
    run_quietly(
        Command::new("systemctl")
            .args(["--user", "unset-environment"])
            .args(WAYLAND_ONLY_VARS),
    );

    let vars = present_session_vars();
    if vars.is_empty() {
        return;
    }
    let mut import = Command::new("dbus-update-activation-environment");
    import.arg("--systemd").args(&vars);
    if !run_quietly(&mut import) {
        // Without systemd support, at least update D-Bus activation.
        run_quietly(Command::new("dbus-update-activation-environment").args(&vars));
    }
}

fn present_session_vars() -> Vec<&'static str> {
    SESSION_VARS
        .iter()
        .copied()
        .filter(|name| env::var_os(name).is_some_and(|value| !value.is_empty()))
        .collect()
}

/// True when `WAYLAND_DISPLAY` names a compositor that accepts connections.
/// A leftover variable pointing at a dead socket is not a live session.
fn host_wayland_session_alive() -> bool {
    let Some(display) = env::var_os("WAYLAND_DISPLAY").filter(|value| !value.is_empty()) else {
        return false;
    };
    let path = PathBuf::from(display);
    let socket = if path.is_absolute() {
        path
    } else {
        match env::var_os("XDG_RUNTIME_DIR") {
            Some(runtime) => PathBuf::from(runtime).join(path),
            None => return false,
        }
    };
    UnixStream::connect(socket).is_ok()
}

/// Run a best-effort helper; returns whether it ran and succeeded.
fn run_quietly(command: &mut Command) -> bool {
    match command.status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            log::debug!("{command:?} exited with status {status}");
            false
        }
        Err(error) => {
            log::debug!("{command:?} unavailable: {error}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_only_vars_are_not_exported() {
        for var in WAYLAND_ONLY_VARS {
            assert!(!SESSION_VARS.contains(var));
        }
        assert!(SESSION_VARS.contains(&"DISPLAY"));
        assert!(SESSION_VARS.contains(&"XDG_SESSION_TYPE"));
    }
}
