use super::{
    CUSTOM_STATUS_RECEIVED, flush_i3bar_click_events, parse_i3bar_header, parse_i3bar_json,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

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
    std::thread::spawn(move || {
        use std::sync::atomic::Ordering;
        use std::thread;
        use std::time::Duration;

        thread::sleep(Duration::from_millis(500));

        loop {
            if CUSTOM_STATUS_RECEIVED.load(Ordering::Relaxed) {
                break;
            }
            super::runtime::send_status_update(&default_status_text());
            thread::sleep(Duration::from_secs(30));
        }
    });
}

pub(crate) fn spawn_status_command(cmd: &str) {
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

        let mut i3bar_mode = false;

        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
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
                        && let Some(mut stdin) = child.stdin.take()
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
    });
}
