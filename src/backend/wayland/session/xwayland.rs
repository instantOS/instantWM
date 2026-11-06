//! Session-owned X11 address with demand-driven, replaceable Xwayland instances.

use crate::backend::wayland::compositor::WaylandState;
use smithay::{
    reexports::{
        calloop::{
            Interest, Mode, PostAction, RegistrationToken,
            generic::Generic,
            timer::{TimeoutAction, Timer},
        },
        wayland_server::Client,
    },
    wayland::selection::{SelectionTarget, current_selection_mime_types},
    xwayland::{X11Wm, XWayland, XWaylandDisplay, XWaylandEvent},
};
use std::{
    process::Stdio,
    time::{Duration, Instant},
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_SECONDS: &str = "10";

/// The display reservation lives for the session, independently of the server.
pub(crate) struct XWaylandSession {
    display: XWaylandDisplay,
    remove_source: Box<dyn Fn(RegistrationToken)>,
    listeners: Vec<RegistrationToken>,
    server: Option<Server>,
    generation: u64,
    failures: u32,
    cooldown: Option<RegistrationToken>,
}

struct Server {
    pid: u32,
    source: RegistrationToken,
    watchdog: RegistrationToken,
    client: Client,
    started: Instant,
    ready: bool,
}

impl XWaylandSession {
    fn reserve(state: &mut WaylandState, display: XWaylandDisplay) -> anyhow::Result<()> {
        let handle = state.native.loop_handle.clone();
        let weak_handle = handle.downgrade();
        let mut session = Self {
            display,
            remove_source: Box::new(move |token| {
                if let Some(handle) = weak_handle.upgrade() {
                    handle.remove(token);
                }
            }),
            listeners: Vec::new(),
            server: None,
            generation: 0,
            failures: 0,
            cooldown: None,
        };
        for socket in session.display.listen_sockets() {
            let token = handle.insert_source(
                Generic::new(socket.try_clone()?, Interest::READ, Mode::Level),
                |_, _, state| {
                    // Defer startup until all callbacks for this event batch have
                    // finished. Both abstract and filesystem sockets may be ready.
                    state.native.loop_handle.insert_idle(start);
                    Ok(PostAction::Disable)
                },
            )?;
            session.listeners.push(token);
        }
        state.native.xdisplay = Some(session.display.display_number());
        state.native.xwayland_session = Some(session);
        Ok(())
    }
}

impl Drop for XWaylandSession {
    fn drop(&mut self) {
        for token in self.listeners.drain(..) {
            (self.remove_source)(token);
        }
        if let Some(server) = self.server.take() {
            (self.remove_source)(server.watchdog);
            (self.remove_source)(server.source);
        }
        if let Some(token) = self.cooldown.take() {
            (self.remove_source)(token);
        }
    }
}

/// Reserve and advertise an X11 address before autostart, without starting a process.
pub(crate) fn setup(state: &mut WaylandState) {
    let result = XWaylandDisplay::new(None, true)
        .map_err(anyhow::Error::from)
        .and_then(|display| XWaylandSession::reserve(state, display));
    match result {
        Ok(()) => {
            let display = state.native.xdisplay.unwrap();
            unsafe {
                std::env::set_var("DISPLAY", format!(":{display}"));
            }
            super::import_env_into_dbus_activation();
            log::info!("Xwayland available on demand at :{display}");
        }
        Err(error) => log::error!("failed to reserve Xwayland display: {error}"),
    }
}

fn start(state: &mut WaylandState) {
    let Some(mut session) = state.native.xwayland_session.take() else {
        return;
    };
    if session.server.is_some() || session.cooldown.is_some() {
        state.native.xwayland_session = Some(session);
        return;
    }
    let handle = state.native.loop_handle.clone();
    for token in &session.listeners {
        if let Err(error) = handle.disable(token) {
            log::warn!("failed to suspend X11 activation socket: {error}");
        }
    }
    session.generation += 1;
    let generation = session.generation;
    let display = session.display.clone();
    state.native.xwayland_session = Some(session);
    let envs: Vec<(&str, &str)> = if cfg!(test) {
        vec![("XWAYLAND_NO_GLAMOR", "1")]
    } else {
        Vec::new()
    };
    let result = XWayland::spawn_with_inherited_env_on_display(
        &state.native.display_handle,
        display,
        envs,
        ["-terminate", IDLE_SECONDS],
        Stdio::null(),
        Stdio::inherit(),
        |_| (),
    );
    let (server, client) = match result {
        Ok(server) => server,
        Err(error) => {
            log::error!("failed to start Xwayland: {error}");
            stop(state, generation, true);
            return;
        }
    };
    let pid = server.pid().expect("new Xwayland process");
    log::info!("starting Xwayland process {pid}");
    let wm_client = client.clone();
    let wm_handle = handle.clone();
    let source = match handle.insert_source(server, move |event, _, state| match event {
        XWaylandEvent::Ready { x11_socket, .. } => {
            match X11Wm::start_wm(
                wm_handle.clone(),
                &state.native.display_handle,
                x11_socket,
                wm_client.clone(),
            ) {
                Ok(mut wm) => {
                    if let Err(error) = wm.allow_idle_termination() {
                        log::error!("Xwayland idle termination requires XFixes 6: {error}");
                        state
                            .native
                            .loop_handle
                            .insert_idle(move |state| stop(state, generation, true));
                        return;
                    }
                    crate::backend::wayland::render::cursor_theme::CursorManager::from_environment(
                        &state.native.cursor_config,
                    )
                    .configure_xwayland(&mut wm);
                    for target in [SelectionTarget::Clipboard, SelectionTarget::Primary] {
                        if let Err(error) = wm.new_selection(
                            target,
                            current_selection_mime_types(&state.native.seat, target),
                        ) {
                            log::warn!("failed to initialize Xwayland selection: {error}");
                        }
                    }
                    state.native.xwm = Some(wm);
                    if let Some(server) = state
                        .native
                        .xwayland_session
                        .as_mut()
                        .and_then(|s| s.server.as_mut())
                    {
                        server.ready = true;
                    }
                    log::info!("Xwayland ready");
                }
                Err(error) => {
                    log::error!("failed to initialize Xwayland window manager: {error}");
                    state
                        .native
                        .loop_handle
                        .insert_idle(move |state| stop(state, generation, true));
                }
            }
        }
        XWaylandEvent::Error => {
            log::error!("Xwayland exited during startup");
            state
                .native
                .loop_handle
                .insert_idle(move |state| stop(state, generation, true));
        }
    }) {
        Ok(source) => source,
        Err(error) => {
            log::error!("failed to register Xwayland: {error}");
            stop(state, generation, true);
            return;
        }
    };
    let watchdog = match handle.insert_source(
        Timer::from_duration(Duration::from_secs(1)),
        move |_, _, state| {
            let server = state
                .native
                .xwayland_session
                .as_ref()
                .filter(|s| s.generation == generation)
                .and_then(|s| s.server.as_ref());
            let Some(server) = server else {
                return TimeoutAction::Drop;
            };
            let dead = server
                .client
                .get_credentials(&state.native.display_handle)
                .is_err();
            let timeout = !server.ready && server.started.elapsed() >= STARTUP_TIMEOUT;
            if dead || timeout {
                if timeout {
                    log::error!("Xwayland startup timed out");
                }
                state
                    .native
                    .loop_handle
                    .insert_idle(move |state| stop(state, generation, timeout));
                TimeoutAction::Drop
            } else {
                TimeoutAction::ToDuration(Duration::from_secs(1))
            }
        },
    ) {
        Ok(token) => token,
        Err(error) => {
            handle.remove(source);
            log::error!("failed to register Xwayland watchdog: {error}");
            stop(state, generation, true);
            return;
        }
    };
    state.native.xwayland_session.as_mut().unwrap().server = Some(Server {
        pid,
        source,
        watchdog,
        client,
        started: Instant::now(),
        ready: false,
    });
}

/// Reconcile all X11 state before rearming activation. Generation checks keep
/// queued callbacks from an old server from tearing down its replacement.
fn stop(state: &mut WaylandState, generation: u64, failed: bool) {
    let Some(mut session) = state.native.xwayland_session.take() else {
        return;
    };
    if session.generation != generation || session.cooldown.is_some() {
        state.native.xwayland_session = Some(session);
        return;
    }
    let handle = state.native.loop_handle.clone();
    let failed = failed
        || session
            .server
            .as_ref()
            .is_none_or(|s| !s.ready || s.started.elapsed() < STARTUP_TIMEOUT);
    if let Some(server) = session.server.take() {
        log::info!("releasing Xwayland process {}", server.pid);
        handle.remove(server.watchdog);
        // Drop the Xwayland source, disconnecting the Wayland client. The session
        // retains the listening sockets and display lock for the next instance.
        handle.remove(server.source);
    }
    if let Some(id) = state.native.xwm.as_ref().map(X11Wm::id) {
        state.cleanup_xwayland_windows(id);
    }
    state.native.xwm = None;
    // DISPLAY describes the session socket, not the currently running process.
    if failed {
        session.failures = session.failures.saturating_add(1);
    } else {
        session.failures = 0;
    }
    let delay = if failed {
        Duration::from_secs(1 << session.failures.min(5))
    } else {
        Duration::ZERO
    };
    state.native.xwayland_session = Some(session);
    match handle.insert_source(Timer::from_duration(delay), move |_, _, state| {
        if let Some(session) = state
            .native
            .xwayland_session
            .as_mut()
            .filter(|s| s.generation == generation)
        {
            session.cooldown = None;
            for token in &session.listeners {
                if let Err(error) = state.native.loop_handle.enable(token) {
                    log::error!("failed to rearm X11 activation: {error}");
                }
            }
        }
        TimeoutAction::Drop
    }) {
        Ok(token) => state.native.xwayland_session.as_mut().unwrap().cooldown = Some(token),
        Err(error) => log::error!("failed to schedule Xwayland recovery: {error}"),
    }
}

pub(crate) fn disconnected(state: &mut WaylandState, id: smithay::xwayland::xwm::XwmId) {
    if state.native.xwm.as_ref().is_some_and(|wm| wm.id() == id)
        && let Some(session) = &state.native.xwayland_session
    {
        let generation = session.generation;
        state
            .native
            .loop_handle
            .insert_idle(move |state| stop(state, generation, false));
    }
}

#[cfg(test)]
mod tests;
