//! Startup and reload configuration diagnostics must reach `instantwmctl status`.
//! Run with `cargo test --test x11_config_errors -- --ignored --nocapture`.
//! Requires Xvfb; uses a private display, HOME and IPC socket.

use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait<T>(description: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(result) = check() {
            return result;
        }
        assert!(Instant::now() < end, "Timed out: {description}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn server(directory: &Path) -> (Process, String) {
    let file = directory.join("display");
    let fd = std::fs::File::create(&file).unwrap();
    unsafe {
        libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, 0);
    }
    let process = Process(
        Command::new("Xvfb")
            .args([
                "-screen",
                "0",
                "800x600x24",
                "-displayfd",
                &fd.as_raw_fd().to_string(),
                "-nolisten",
                "tcp",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let number = wait("X server startup", || {
        let text = std::fs::read_to_string(&file).ok()?;
        (!text.trim().is_empty()).then(|| text.trim().to_owned())
    });
    (process, format!(":{number}"))
}

fn write_config(home: &Path, body: &str) {
    let dir = home.join(".config/instantwm");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), body).unwrap();
}

/// Prevent startup from probing for the optional `ins` application aliases.
fn install_default_aliases(home: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let dir = home.join(".config/instantos/default");
    std::fs::create_dir_all(&dir).unwrap();
    for alias in [
        "terminal",
        "termfilemanager",
        "filemanager",
        "lockscreen",
        "browser",
        "editor",
        "systemmonitor",
    ] {
        let path = dir.join(alias);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
    }
}

fn ctl(exe: &str, display: &str, socket: &Path, args: &[&str]) -> std::process::Output {
    Command::new(exe)
        .env("DISPLAY", display)
        .env("INSTANTWM_SOCKET", socket)
        .args(args)
        .output()
        .unwrap()
}

fn status_json(exe: &str, display: &str, socket: &Path) -> serde_json::Value {
    let output = ctl(exe, display, socket, &["--json", "status"]);
    assert!(
        output.status.success(),
        "status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("status response must be JSON")
}

#[test]
#[ignore = "requires a private Xvfb server"]
fn startup_and_reload_config_errors_reach_status() {
    let exe = env!("CARGO_BIN_EXE_instantwm");
    let ctl_exe = env!("CARGO_BIN_EXE_instantwmctl");
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    install_default_aliases(&home);
    write_config(&home, "[bar]\nshow = false\n\n[layout]\ninner_gap = -5\n");
    let (_server, display) = server(dir.path());
    let socket = dir.path().join("iwm.sock");

    let _wm = Process(
        Command::new(exe)
            .env("DISPLAY", &display)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("INSTANTWM_AUTOSTART", "0")
            .env("INSTANTWM_SOCKET_BIND", &socket)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait("WM startup", || {
        ctl(ctl_exe, &display, &socket, &["status"])
            .status
            .success()
            .then_some(())
    });

    let status = status_json(ctl_exe, &display, &socket);
    let errors = status["config_errors"]
        .as_array()
        .expect("config_errors must be an array");
    assert!(!errors.is_empty(), "{status}");
    assert!(
        errors.iter().any(|error| error
            .as_str()
            .is_some_and(|text| text.contains("inner_gap"))),
        "{status}"
    );

    write_config(&home, "[bar]\nshow = false\n");
    let reload = ctl(ctl_exe, &display, &socket, &["reload"]);
    assert!(
        reload.status.success(),
        "reload failed: {}",
        String::from_utf8_lossy(&reload.stderr)
    );
    let status = status_json(ctl_exe, &display, &socket);
    assert_eq!(status["config_errors"], serde_json::json!([]), "{status}");
}
