use super::{
    CUSTOM_STATUS_RECEIVED, flush_i3bar_click_events, parse_i3bar_header, parse_i3bar_json,
};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq)]
enum StatusSourceKind {
    Default,
    Command(String),
}

#[derive(Debug)]
struct RunningStatusSource {
    kind: StatusSourceKind,
    stop: Arc<AtomicBool>,
    pid: Arc<AtomicU32>,
}

impl RunningStatusSource {
    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        let pid = self.pid.load(Ordering::Acquire) as i32;
        if pid > 0 {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

static STATUS_SOURCE: OnceLock<Mutex<Option<RunningStatusSource>>> = OnceLock::new();

fn status_source() -> &'static Mutex<Option<RunningStatusSource>> {
    STATUS_SOURCE.get_or_init(|| Mutex::new(None))
}

fn set_status_source(next: StatusSourceKind) -> Option<(Arc<AtomicBool>, Arc<AtomicU32>)> {
    let mut active = status_source().lock().ok()?;

    if active.as_ref().is_some_and(|source| source.kind == next) {
        return None;
    }

    if let Some(source) = active.take() {
        source.stop();
    }

    let stop = Arc::new(AtomicBool::new(false));
    let pid = Arc::new(AtomicU32::new(0));

    *active = Some(RunningStatusSource {
        kind: next,
        stop: Arc::clone(&stop),
        pid: Arc::clone(&pid),
    });

    Some((stop, pid))
}

fn default_status_text() -> String {
    use std::time::SystemTime;

    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let time_str = unsafe {
        let secs_i64 = secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs_i64, &mut tm);
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
    };

    format!("instantwm-{VERSION} {time_str}")
}

/// Spawn a background thread that periodically sends the default status
/// (version + current time) via IPC. Used when no `status_command` is configured.
pub(crate) fn spawn_default_status() {
    let Some((stop, _)) = set_status_source(StatusSourceKind::Default) else {
        return;
    };

    CUSTOM_STATUS_RECEIVED.store(false, Ordering::Relaxed);

    std::thread::spawn(move || {
        use std::thread;
        use std::time::Duration;

        thread::sleep(Duration::from_millis(500));

        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            if CUSTOM_STATUS_RECEIVED.load(Ordering::Relaxed) {
                break;
            }
            super::runtime::send_status_update(&default_status_text());
            thread::sleep(Duration::from_secs(30));
        }
    });
}

pub(crate) fn spawn_status_command(cmd: &str) {
    let Some((stop, pid)) = set_status_source(StatusSourceKind::Command(cmd.to_string())) else {
        return;
    };

    CUSTOM_STATUS_RECEIVED.store(false, Ordering::Relaxed);

    let cmd_str = cmd.to_string();
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};

        let mut child = match Command::new("sh")
            .arg("-c")
            .arg(&cmd_str)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
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

        pid.store(child.id(), Ordering::Release);

        if stop.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }

        let stdout = child.stdout.take();
        let mut child_stdin = child.stdin.take();

        let mut i3bar_mode = false;

        if let Some(stdout) = stdout {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if stop.load(Ordering::Relaxed) {
                    break;
                }

                let Ok(line) = line else {
                    continue;
                };

                let text = line.trim();
                if text.is_empty() || text == "[" {
                    continue;
                }

                if !i3bar_mode && let Some(header) = parse_i3bar_header(text) {
                    i3bar_mode = true;
                    if header.click_events
                        && let Some(mut stdin) = child_stdin.take()
                    {
                        std::thread::spawn(move || {
                            let mut first_click_event = true;
                            while flush_i3bar_click_events(&mut stdin, &mut first_click_event)
                                .is_ok()
                            {
                                std::thread::sleep(std::time::Duration::from_millis(25));
                            }
                        });
                    }
                    continue;
                }

                if i3bar_mode {
                    if parse_i3bar_json(text.as_bytes()).is_some() {
                        super::runtime::send_status_update(text);
                    } else {
                        log::debug!("dropping malformed i3bar status frame: {text}");
                    }
                } else {
                    super::runtime::send_status_update(text);
                }
            }
        }

        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
        }
        let _ = child.wait();
    });
}

pub(crate) fn reload_status_command(previous: Option<&str>, next: Option<&str>) {
    if previous == next {
        return;
    }

    if let Some(cmd) = next {
        spawn_status_command(cmd);
    } else {
        spawn_default_status();
    }
}
