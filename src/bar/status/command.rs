use super::parse::{parse_i3bar_json, parse_status, plain_text_status};
use super::{
    I3BarHeader, I3BarSignals, I3ClickEvent, StatusBlocks, parse_i3bar_header,
    runtime::write_i3bar_click_event,
};
use calloop::ping::{Ping, PingSource};
use std::io::Write;
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_SUSPENSION_SIGNALS: I3BarSignals = I3BarSignals {
    stop: libc::SIGSTOP,
    resume: libc::SIGCONT,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum StatusSourceKind {
    Default,
    Command(String),
}

#[derive(Debug)]
struct RunningStatusSource {
    kind: StatusSourceKind,
    process: Arc<StatusProcess>,
    updates: Receiver<StatusUpdate>,
}

#[derive(Debug)]
pub(super) struct StatusUpdate {
    pub blocks: StatusBlocks,
    pub click_events: bool,
}

/// Worker-side end of a status source's update stream.
struct StatusSink {
    updates: Sender<StatusUpdate>,
    wake: Option<Ping>,
}

impl StatusSink {
    fn send(&self, blocks: StatusBlocks, click_events: bool) {
        if self
            .updates
            .send(StatusUpdate {
                blocks,
                click_events,
            })
            .is_ok()
            && let Some(wake) = self.wake.as_ref()
        {
            wake.ping();
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum SuspensionPolicy {
    #[default]
    Undecided,
    Disabled,
    Signals(I3BarSignals),
}

#[derive(Debug, Default)]
struct StatusProcessState {
    pid: Option<i32>,
    suspension: SuspensionPolicy,
    visible: bool,
    suspended: bool,
    click_sender: Option<Sender<I3ClickEvent>>,
}

#[derive(Debug)]
struct StatusProcess {
    stopped: AtomicBool,
    state: Mutex<StatusProcessState>,
}

impl StatusProcess {
    fn new(visible: bool) -> Self {
        Self {
            stopped: AtomicBool::new(false),
            state: Mutex::new(StatusProcessState {
                visible,
                ..StatusProcessState::default()
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, StatusProcessState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    fn set_pid(&self, pid: u32) -> bool {
        let Ok(pid) = i32::try_from(pid) else {
            return false;
        };
        let mut state = self.state();
        if self.is_stopped() {
            return false;
        }
        state.pid = Some(pid);
        reconcile_suspension(&mut state);
        true
    }

    fn configure_i3bar(
        &self,
        header: &I3BarHeader,
        click_sender: Option<Sender<I3ClickEvent>>,
    ) -> bool {
        let mut state = self.state();
        if self.is_stopped() {
            return false;
        }
        state.suspension = header
            .suspension
            .map_or(SuspensionPolicy::Disabled, SuspensionPolicy::Signals);
        state.click_sender = click_sender;
        reconcile_suspension(&mut state);
        true
    }

    fn configure_plain_text(&self) -> bool {
        let mut state = self.state();
        if self.is_stopped() {
            return false;
        }
        state.suspension = SuspensionPolicy::Signals(DEFAULT_SUSPENSION_SIGNALS);
        reconcile_suspension(&mut state);
        true
    }

    fn set_visible(&self, visible: bool) {
        let mut state = self.state();
        if state.visible == visible {
            return;
        }
        state.visible = visible;
        reconcile_suspension(&mut state);
    }

    fn enqueue_click(&self, event: I3ClickEvent) {
        let state = self.state();
        if let Some(sender) = state.click_sender.as_ref() {
            let _ = sender.send(event);
        }
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        let mut state = self.state();
        state.click_sender = None;
        if let Some(pid) = state.pid {
            // The command is launched in its own process group. Kill the whole
            // pipeline so `sh -c` cannot leave a status producer behind.
            kill_status_process_group_id(pid);
        }
    }

    fn clear_pid(&self, pid: u32) {
        let Ok(pid) = i32::try_from(pid) else {
            return;
        };
        let mut state = self.state();
        if state.pid == Some(pid) {
            state.pid = None;
            state.suspension = SuspensionPolicy::Undecided;
            state.suspended = false;
            state.click_sender = None;
        }
    }
}

fn pending_suspension_signal(state: &StatusProcessState) -> Option<(i32, bool)> {
    state.pid?;
    let SuspensionPolicy::Signals(signals) = state.suspension else {
        return None;
    };
    match (state.visible, state.suspended) {
        (false, false) => Some((signals.stop, true)),
        (true, true) => Some((signals.resume, false)),
        _ => None,
    }
}

fn reconcile_suspension(state: &mut StatusProcessState) {
    let Some((signal, suspended)) = pending_suspension_signal(state) else {
        return;
    };
    let Some(pid) = state.pid else {
        return;
    };

    let SuspensionPolicy::Signals(signals) = state.suspension else {
        return;
    };
    let target = signal_target(pid, signals);
    if unsafe { libc::kill(target, signal) } == 0 {
        state.suspended = suspended;
        return;
    }

    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        state.pid = None;
        state.click_sender = None;
        state.suspended = false;
    } else {
        log::warn!("failed to signal status command target {target} with {signal}: {error}");
    }
}

fn signal_target(pid: i32, signals: I3BarSignals) -> i32 {
    // SIGSTOP/SIGCONT are safe to broadcast to the isolated command group and
    // must reach subprocesses doing the actual work behind `sh -c`. Custom
    // signals retain i3bar's single-process semantics because helpers may not
    // share the status program's custom signal handlers.
    if signals == DEFAULT_SUSPENSION_SIGNALS {
        -pid
    } else {
        pid
    }
}

/// The active status producer, owned by the bar.
///
/// Every source gets its own update channel, so frames from a replaced
/// source can never reach the bar.
#[derive(Debug)]
pub(crate) struct StatusSources {
    active: Option<RunningStatusSource>,
    visible: bool,
    wake: Option<Ping>,
    wake_source: Option<PingSource>,
}

impl Default for StatusSources {
    fn default() -> Self {
        let (wake, wake_source) = match calloop::ping::make_ping() {
            Ok((wake, source)) => (Some(wake), Some(source)),
            Err(error) => {
                log::warn!("status updates cannot wake the event loop: {error}");
                (None, None)
            }
        };
        Self {
            active: None,
            visible: true,
            wake,
            wake_source,
        }
    }
}

impl Drop for StatusSources {
    fn drop(&mut self) {
        if let Some(source) = self.active.take() {
            source.process.stop();
        }
    }
}

impl StatusSources {
    /// Event-loop source woken by status frames. Frames sent before it is
    /// registered still wake the loop once it is.
    pub(crate) fn take_wake_source(&mut self) -> Option<PingSource> {
        self.wake_source.take()
    }

    /// Run the configured status command, the auto-detected `i3status-rs`,
    /// or the built-in clock, in that order of precedence.
    pub(crate) fn start(&mut self, command: Option<&str>) {
        match command {
            Some(command) => self.spawn_command(command),
            None if is_i3status_rs_available() => self.spawn_command("i3status-rs"),
            None => self.spawn_default(),
        }
    }

    fn replace(&mut self, kind: StatusSourceKind) -> Option<(Arc<StatusProcess>, StatusSink)> {
        if self
            .active
            .as_ref()
            .is_some_and(|source| source.kind == kind)
        {
            return None;
        }
        if let Some(source) = self.active.take() {
            source.process.stop();
        }

        let process = Arc::new(StatusProcess::new(self.visible));
        let (sender, updates) = mpsc::channel();
        self.active = Some(RunningStatusSource {
            kind,
            process: Arc::clone(&process),
            updates,
        });
        Some((
            process,
            StatusSink {
                updates: sender,
                wake: self.wake.clone(),
            },
        ))
    }

    pub(super) fn set_visible(&mut self, visible: bool) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if let Some(source) = self.active.as_ref() {
            source.process.set_visible(visible);
        }
    }

    pub(crate) fn send_click(&self, event: I3ClickEvent) {
        if let Some(source) = self.active.as_ref() {
            source.process.enqueue_click(event);
        }
    }

    pub(super) fn take_latest_update(&self) -> Option<StatusUpdate> {
        self.active.as_ref()?.updates.try_iter().last()
    }

    pub(super) fn stop_default_source(&mut self) {
        if self
            .active
            .as_ref()
            .is_some_and(|source| source.kind == StatusSourceKind::Default)
            && let Some(source) = self.active.take()
        {
            source.process.stop();
        }
    }

    /// Periodically publish the version and current time.
    fn spawn_default(&mut self) {
        let Some((process, sink)) = self.replace(StatusSourceKind::Default) else {
            return;
        };

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(500));

            while !process.is_stopped() {
                sink.send(plain_text_status(&default_status_text()), false);
                thread::sleep(Duration::from_secs(30));
            }
        });
    }

    fn spawn_command(&mut self, cmd: &str) {
        let Some((process, sink)) = self.replace(StatusSourceKind::Command(cmd.to_string())) else {
            return;
        };
        let cmd_str = cmd.to_string();
        thread::spawn(move || run_status_command(&cmd_str, &process, &sink));
    }
}

pub(crate) fn sync_visibility(wm: &mut crate::wm::Wm) {
    let visible = status_visible(&wm.core.model);
    wm.bar.status_sources.set_visible(visible);
}

fn status_visible(model: &crate::model::WmModel) -> bool {
    model
        .selected_monitor()
        .is_some_and(|monitor| monitor.bar_visible(&model.clients))
}

fn default_status_text() -> String {
    use std::time::SystemTime;

    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let time_str = unsafe {
        let secs_i64 = secs as libc::time_t;
        let mut tm: libc::tm = mem::zeroed();
        libc::localtime_r(&secs_i64, &mut tm);
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
    };

    format!("instantwm-{VERSION} {time_str}")
}

fn run_status_command(cmd_str: &str, process: &StatusProcess, sink: &StatusSink) {
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(cmd_str)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .process_group(0)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "instantwm: failed to spawn status_command '{}': {}",
                cmd_str, e
            );
            return;
        }
    };

    if !process.set_pid(child.id()) {
        kill_status_process_group(&mut child);
        let _ = child.wait();
        return;
    }

    let stdout = child.stdout.take();
    let mut child_stdin = child.stdin.take();

    #[derive(Clone, Copy)]
    enum CommandProtocol {
        Undecided,
        PlainText,
        I3Bar { click_events: bool },
    }

    let mut protocol = CommandProtocol::Undecided;

    if let Some(stdout) = stdout {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if process.is_stopped() {
                break;
            }

            let Ok(line) = line else {
                continue;
            };

            let text = line.trim();
            if text.is_empty() {
                continue;
            }

            if matches!(protocol, CommandProtocol::Undecided) {
                if let Some(header) = parse_i3bar_header(text) {
                    protocol = CommandProtocol::I3Bar {
                        click_events: header.click_events,
                    };
                    let click_channel = header.click_events.then(mpsc::channel::<I3ClickEvent>);
                    let click_sender = click_channel.as_ref().map(|(sender, _)| sender.clone());
                    if process.configure_i3bar(&header, click_sender)
                        && let (Some(mut stdin), Some((_, receiver))) =
                            (child_stdin.take(), click_channel)
                    {
                        thread::spawn(move || write_click_events(&mut stdin, receiver));
                    }
                    continue;
                }

                protocol = CommandProtocol::PlainText;
                if !process.configure_plain_text() {
                    break;
                }
            }

            match protocol {
                CommandProtocol::I3Bar { .. } if text == "[" => {}
                CommandProtocol::I3Bar { click_events } => match parse_i3bar_json(text) {
                    Some(blocks) => sink.send(blocks, click_events),
                    None => log::debug!("dropping malformed i3bar status frame: {text}"),
                },
                CommandProtocol::PlainText => {
                    sink.send(parse_status(text), false);
                }
                CommandProtocol::Undecided => unreachable!("protocol was classified above"),
            }
        }
    }

    // Once stdout closes, this source can no longer provide status. Terminate it
    // before clearing the shared PID so that the PID cannot be recycled while it
    // is still signalable through `StatusProcess`.
    kill_status_process_group(&mut child);
    process.clear_pid(child.id());
    let _ = child.wait();
}

fn kill_status_process_group(child: &mut std::process::Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        kill_status_process_group_id(pid);
    } else {
        let _ = child.kill();
    }
}

fn kill_status_process_group_id(pid: i32) {
    if !crate::util::signal_process_group(pid, libc::SIGKILL) {
        // A failed group lookup must not turn shutdown/reload into a leaked
        // shell. The unreaped child still owns this PID, so fallback is safe.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
}

fn write_click_events(writer: &mut impl Write, receiver: mpsc::Receiver<I3ClickEvent>) {
    let mut first_event = true;
    for event in receiver {
        if write_i3bar_click_event(&mut *writer, &event, &mut first_event)
            .and_then(|()| writer.flush())
            .is_err()
        {
            break;
        }
    }
}

fn is_i3status_rs_available() -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join("i3status-rs").is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Monitor, Rect};

    fn signals() -> I3BarSignals {
        I3BarSignals {
            stop: libc::SIGSTOP,
            resume: libc::SIGCONT,
        }
    }

    fn click_event(button: u8) -> I3ClickEvent {
        I3ClickEvent {
            name: None,
            instance: None,
            button,
            x: 0,
            y: 0,
            relative_x: 0,
            relative_y: 0,
            output_x: 0,
            output_y: 0,
            width: 1,
            height: 1,
            modifiers: Vec::new(),
        }
    }

    #[test]
    fn suspension_transition_only_occurs_when_state_changes() {
        let mut state = StatusProcessState {
            pid: Some(42),
            suspension: SuspensionPolicy::Signals(signals()),
            visible: false,
            ..StatusProcessState::default()
        };

        assert_eq!(
            pending_suspension_signal(&state),
            Some((libc::SIGSTOP, true))
        );
        state.suspended = true;
        assert_eq!(pending_suspension_signal(&state), None);

        state.visible = true;
        assert_eq!(
            pending_suspension_signal(&state),
            Some((libc::SIGCONT, false))
        );
        state.suspended = false;
        assert_eq!(pending_suspension_signal(&state), None);
    }

    #[test]
    fn suspension_requires_a_live_pid_and_completed_protocol_negotiation() {
        let mut state = StatusProcessState {
            visible: false,
            suspension: SuspensionPolicy::Signals(signals()),
            ..StatusProcessState::default()
        };
        assert_eq!(pending_suspension_signal(&state), None);

        state.pid = Some(42);
        state.suspension = SuspensionPolicy::Undecided;
        assert_eq!(pending_suspension_signal(&state), None);
        state.suspension = SuspensionPolicy::Disabled;
        assert_eq!(pending_suspension_signal(&state), None);
    }

    #[test]
    fn plain_text_enables_defaults_only_after_the_first_line_classifies_it() {
        let process = StatusProcess::new(false);
        assert_eq!(process.state().suspension, SuspensionPolicy::Undecided);

        assert!(process.configure_plain_text());
        assert_eq!(
            process.state().suspension,
            SuspensionPolicy::Signals(DEFAULT_SUSPENSION_SIGNALS)
        );
    }

    #[test]
    fn i3bar_opt_out_remains_active_when_the_bar_starts_hidden() {
        let process = StatusProcess::new(false);
        let header = I3BarHeader {
            click_events: false,
            suspension: None,
        };

        assert!(process.configure_i3bar(&header, None));
        let state = process.state();
        assert_eq!(state.suspension, SuspensionPolicy::Disabled);
        assert_eq!(pending_suspension_signal(&state), None);
    }

    #[test]
    fn default_suspension_targets_the_status_process_group() {
        assert_eq!(signal_target(42, signals()), -42);
        assert_eq!(
            signal_target(
                42,
                I3BarSignals {
                    stop: libc::SIGUSR1,
                    resume: libc::SIGUSR2,
                }
            ),
            42
        );
    }

    #[test]
    fn click_writer_consumes_its_channel_until_disconnected() {
        let (sender, receiver) = mpsc::channel();
        sender.send(click_event(1)).unwrap();
        sender.send(click_event(3)).unwrap();
        drop(sender);

        let mut output = Vec::new();
        write_click_events(&mut output, receiver);
        let output = String::from_utf8(output).unwrap();

        assert!(output.starts_with("[\n{"));
        assert!(output.contains("\n,\n{"));
        assert!(output.contains("\"button\":1"));
        assert!(output.contains("\"button\":3"));
    }

    #[test]
    fn frames_from_a_replaced_source_never_reach_the_bar() {
        let mut sources = StatusSources::default();
        let (_, old) = sources
            .replace(StatusSourceKind::Command("old".to_string()))
            .unwrap();
        let (_, current) = sources
            .replace(StatusSourceKind::Command("new".to_string()))
            .unwrap();

        old.send(plain_text_status("old"), false);
        assert!(sources.take_latest_update().is_none());

        current.send(plain_text_status("first"), false);
        current.send(plain_text_status("second"), true);
        let latest = sources.take_latest_update().unwrap();
        assert_eq!(latest.blocks[0].full_text, "second");
        assert!(latest.click_events);
    }

    #[test]
    fn default_status_keeps_updating_until_an_external_override() {
        let mut bar = crate::bar::BarState::default();
        let (_, sink) = bar
            .status_sources
            .replace(StatusSourceKind::Default)
            .unwrap();

        sink.send(plain_text_status("first"), false);
        assert!(bar.drain_status_updates());
        assert_eq!(bar.runtime.status[0].full_text, "first");
        assert!(bar.status_sources.active.is_some());

        sink.send(plain_text_status("second"), false);
        assert!(bar.drain_status_updates());
        assert_eq!(bar.runtime.status[0].full_text, "second");

        bar.set_status_text("second");
        assert!(bar.status_sources.active.is_none());
    }

    #[test]
    fn status_visibility_follows_the_selected_monitor_only() {
        let mut model = crate::model::WmModel::default();
        let mut hidden = Monitor::new_with_values(false);
        hidden.monitor_rect = Rect::new(0, 0, 100, 100);
        let hidden_id = model.monitors.push(hidden);
        let mut visible = Monitor::new_with_values(true);
        visible.monitor_rect = Rect::new(100, 0, 100, 100);
        let visible_id = model.monitors.push(visible);

        model.monitors.set_selected(hidden_id);
        assert!(!status_visible(&model));
        model.monitors.set_selected(visible_id);
        assert!(status_visible(&model));
    }
}
