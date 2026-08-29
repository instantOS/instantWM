//! Wayland session environment, socket, and XWayland lifecycle.

use std::env;
use std::process::{Command, Stdio};
use std::sync::Arc;

use smithay::reexports::calloop::LoopHandle;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::xwayland::{X11Wm, XWayland, XWaylandEvent};

use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};

// ─────────────────────────────────────────────────────────────────────────────
// Session environment
// ─────────────────────────────────────────────────────────────────────────────

/// Set the standard environment variables that tell toolkit clients how to
/// connect to this compositor.
///
/// Called after the Wayland socket name is known.  Both the nested backend
/// (which merely exports `WAYLAND_DISPLAY` into the nested environment) and
/// the standalone DRM backend (which is the actual session compositor) use the
/// same set of variables.
pub fn apply_session_env(socket_name: &str) {
    unsafe {
        env::set_var("WAYLAND_DISPLAY", socket_name);
        env::set_var("XDG_SESSION_TYPE", "wayland");
        env::set_var("XDG_CURRENT_DESKTOP", "instantwm");
        env::set_var("XDG_SESSION_DESKTOP", "instantwm");
        env::set_var("DESKTOP_SESSION", "instantwm");
        env::remove_var("DISPLAY");
        env::set_var("GDK_BACKEND", "wayland");
        env::set_var("QT_QPA_PLATFORM", "wayland");
        env::set_var("SDL_VIDEODRIVER", "wayland");
        env::set_var("CLUTTER_BACKEND", "wayland");
    }
}

pub fn ensure_dbus_session() {
    if env::var("DBUS_SESSION_BUS_ADDRESS").is_ok() {
        return;
    }

    let Ok(output) = Command::new("dbus-daemon")
        .arg("--session")
        .arg("--fork")
        .arg("--print-address=1")
        .arg("--nopidfile")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    else {
        log::warn!("dbus-daemon not found, D-Bus session bus unavailable");
        return;
    };

    let addr = String::from_utf8_lossy(&output.stdout);
    let addr = addr.trim();
    if !addr.is_empty() {
        unsafe { env::set_var("DBUS_SESSION_BUS_ADDRESS", addr) };
        log::info!("Started D-Bus session bus: {addr}");
    }
}

/// Resolve the list of session environment variables to import into D-Bus activation.
pub fn dbus_activation_vars() -> Vec<&'static str> {
    let mut vars = vec![
        "WAYLAND_DISPLAY",
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "XDG_SESSION_TYPE",
        "DESKTOP_SESSION",
    ];
    if env::var_os("DISPLAY").is_some() {
        vars.push("DISPLAY");
    }
    vars
}

/// Reset any portal services leftover from a previous or crashed session so
/// that xdg-desktop-portal cleanly re-activates its backends with the fresh
/// WAYLAND_DISPLAY and environment. Only do this for standalone DRM sessions
/// so nested instances do not disrupt the host desktop's portal services.
pub fn reset_portal_services() {
    if env::var("INSTANTWM_BACKEND").ok().as_deref() == Some("wayland-drm") {
        let _ = Command::new("systemctl")
            .args([
                "--user",
                "stop",
                "xdg-desktop-portal-wlr",
                "xdg-desktop-portal-gtk",
                "xdg-desktop-portal",
            ])
            .status();
    }
}

/// Import the Wayland session environment into the D-Bus activation environment.
///
/// Portals and other D-Bus-activated services need these variables to discover
/// the compositor socket and desktop identity. This mirrors the environment
/// import step commonly done by compositor session wrappers.
pub fn import_env_into_dbus_activation() {
    let vars = dbus_activation_vars();

    let mut attempted = false;
    let mut cmd = Command::new("dbus-update-activation-environment");
    cmd.arg("--systemd");
    for var in &vars {
        cmd.arg(var);
    }

    if let Ok(status) = cmd.status() {
        attempted = true;
        if !status.success() {
            log::debug!(
                "dbus-update-activation-environment exited with status {}",
                status
            );
        }
    }

    // Fall back to the non-systemd import path when systemd integration is
    // unavailable.
    if !attempted {
        let mut cmd = Command::new("dbus-update-activation-environment");
        for var in &vars {
            cmd.arg(var);
        }
        match cmd.status() {
            Ok(status) if !status.success() => log::debug!(
                "dbus-update-activation-environment exited with status {}",
                status
            ),
            Ok(_) => {}
            Err(err) => log::debug!("dbus-update-activation-environment unavailable: {}", err),
        }
    }
}

/// Announce a standalone instantWM session to systemd user services.
///
/// Services such as portals and clipboard managers bind themselves to
/// `graphical-session.target`. Display managers do not universally start that
/// target for custom compositor desktop files, so the session compositor owns
/// the lifecycle explicitly.
pub fn start_graphical_session_target() {
    if env::var("INSTANTWM_BACKEND").ok().as_deref() != Some("wayland-drm") {
        return;
    }
    match Command::new("systemctl")
        .args(["--user", "start", "instantwm-session.target"])
        .status()
    {
        Ok(status) if !status.success() => {
            log::warn!("Failed to start instantwm-session.target: {status}");
        }
        Err(error) => log::debug!("systemctl unavailable for graphical session startup: {error}"),
        Ok(_) => {}
    }
}

/// End the systemd graphical-session lifecycle on a clean compositor exit.
pub fn stop_graphical_session_target() {
    if env::var("INSTANTWM_BACKEND").ok().as_deref() != Some("wayland-drm") {
        return;
    }
    match Command::new("systemctl")
        .args([
            "--user",
            "stop",
            "instantwm-session.target",
            "graphical-session.target",
        ])
        .status()
    {
        Ok(status) if !status.success() => {
            log::warn!("Failed to stop graphical-session.target: {status}");
        }
        Err(error) => log::debug!("systemctl unavailable for graphical session shutdown: {error}"),
        Ok(_) => {}
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Wayland socket
// ─────────────────────────────────────────────────────────────────────────────

/// Create an auto-named Wayland listening socket, register it with the calloop
/// event loop so that new client connections are accepted automatically, and
/// apply the session environment.
///
/// Returns the socket name (e.g. `"wayland-1"`) so callers can log it or pass
/// it to child processes.
pub fn setup_socket(
    loop_handle: &LoopHandle<'static, WaylandState>,
    state: &WaylandState,
) -> String {
    let listening_socket = ListeningSocketSource::new_auto().expect("wayland socket");
    let socket_name = listening_socket
        .socket_name()
        .to_string_lossy()
        .into_owned();

    apply_session_env(&socket_name);
    import_env_into_dbus_activation();
    reset_portal_services();
    start_graphical_session_target();

    loop_handle
        .insert_source(listening_socket, |client, _, data| {
            let _ = data
                .display_handle
                .insert_client(client, Arc::new(WaylandClientState::default()));
        })
        .expect("listening socket source");

    let _ = state; // reserved for future use (e.g. security policy)
    socket_name
}

// ─────────────────────────────────────────────────────────────────────────────
// XWayland
// ─────────────────────────────────────────────────────────────────────────────

/// Spawn XWayland and wire its calloop source into the event loop.
///
/// On success, `DISPLAY` is immediately set to the pre-assigned display number
/// so that any autostart processes that check the environment see it right away.
/// The definitive `DISPLAY` value is set again inside the `XWaylandEvent::Ready`
/// callback once XWayland confirms its display number.
///
/// Errors are logged and silently swallowed: a missing XWayland is non-fatal
/// (pure Wayland clients still work).
pub fn spawn_xwayland(state: &WaylandState, loop_handle: &LoopHandle<'static, WaylandState>) {
    match XWayland::spawn(
        &state.display_handle,
        None,
        std::iter::empty::<(String, String)>(),
        std::iter::empty::<String>(),
        true,
        Stdio::null(),
        Stdio::null(),
        |_| (),
    ) {
        Ok((xwayland, client)) => {
            unsafe { env::set_var("DISPLAY", format!(":{}", xwayland.display_number())) };
            import_env_into_dbus_activation();
            let handle_for_wm = loop_handle.clone();
            if let Err(err) = loop_handle.insert_source(xwayland, move |event, _, data| match event
            {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => {
                    data.xdisplay = Some(display_number);
                    unsafe { env::set_var("DISPLAY", format!(":{display_number}")) };
                    import_env_into_dbus_activation();
                    match X11Wm::start_wm(
                        handle_for_wm.clone(),
                        &data.display_handle,
                        x11_socket,
                        client.clone(),
                    ) {
                        Ok(wm) => data.xwm = Some(wm),
                        Err(e) => log::error!("failed to start X11 WM for XWayland: {e}"),
                    }
                }
                XWaylandEvent::Error => {
                    log::error!("XWayland failed to start");
                }
            }) {
                log::error!("failed to insert XWayland source: {err}");
            }
        }
        Err(err) => {
            log::warn!("failed to spawn XWayland: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dbus_activation_vars_without_display() {
        unsafe { env::remove_var("DISPLAY") };
        let vars = dbus_activation_vars();
        assert!(vars.contains(&"WAYLAND_DISPLAY"));
        assert!(vars.contains(&"XDG_CURRENT_DESKTOP"));
        assert!(!vars.contains(&"DISPLAY"));
    }

    #[test]
    fn test_dbus_activation_vars_with_display() {
        unsafe { env::set_var("DISPLAY", ":0") };
        let vars = dbus_activation_vars();
        assert!(vars.contains(&"WAYLAND_DISPLAY"));
        assert!(vars.contains(&"DISPLAY"));
    }
}
