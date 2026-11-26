//! A second instantwm instance must refuse to start on a display that already
//! has a window manager. Run with
//! `cargo test --test x11_single_instance -- --ignored --nocapture`.
//! Requires Xvfb; uses a private display and does not touch the running desktop.
use std::io::Read;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

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

fn wm_command(exe: &str, display: &str, home: &Path, socket: &Path) -> Command {
    let mut command = Command::new(exe);
    command
        .env("DISPLAY", display)
        .env("HOME", home)
        .env("INSTANTWM_SOCKET_BIND", socket)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn supporting_wm_check(conn: &RustConnection, root: Window, atom: Atom) -> Option<Window> {
    conn.get_property(false, root, atom, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()
}

#[test]
#[ignore = "requires a private Xvfb server"]
fn second_wm_exits_without_touching_root_properties() {
    let exe = env!("CARGO_BIN_EXE_instantwm");
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let (_server, display) = server(dir.path());

    let mut first = Process(
        wm_command(exe, &display, &home, &dir.path().join("wm1.sock"))
            .spawn()
            .unwrap(),
    );

    let (conn, screen_num) = wait("X connection", || {
        RustConnection::connect(Some(&display)).ok()
    });
    let root = conn.setup().roots[screen_num].root;
    let check_atom = conn
        .intern_atom(false, b"_NET_SUPPORTING_WM_CHECK")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let supported_atom = conn
        .intern_atom(false, b"_NET_SUPPORTED")
        .unwrap()
        .reply()
        .unwrap()
        .atom;

    let check_window = wait("first WM advertises _NET_SUPPORTING_WM_CHECK", || {
        supporting_wm_check(&conn, root, check_atom)
    });
    let supported = conn
        .get_property(false, root, supported_atom, AtomEnum::ATOM, 0, 64)
        .unwrap()
        .reply()
        .unwrap()
        .value32()
        .map(|atoms| atoms.collect::<Vec<_>>())
        .unwrap_or_default();
    assert!(!supported.is_empty(), "_NET_SUPPORTED must be populated");

    let mut second = Process({
        let mut command = wm_command(exe, &display, &home, &dir.path().join("wm2.sock"));
        command.stderr(Stdio::piped());
        command.spawn().unwrap()
    });
    let status = wait("second WM to exit", || second.0.try_wait().unwrap());
    assert!(
        !status.success(),
        "a second WM must not start successfully alongside the first"
    );
    let mut stderr = String::new();
    second
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(
        stderr.contains("another window manager is already running"),
        "unexpected second-WM output: {stderr}"
    );

    assert_eq!(
        supporting_wm_check(&conn, root, check_atom),
        Some(check_window),
        "the rejected WM must not replace the EWMH check window"
    );
    let supported_after = conn
        .get_property(false, root, supported_atom, AtomEnum::ATOM, 0, 64)
        .unwrap()
        .reply()
        .unwrap()
        .value32()
        .map(|atoms| atoms.collect::<Vec<_>>())
        .unwrap_or_default();
    assert_eq!(
        supported_after, supported,
        "the rejected WM must not rewrite _NET_SUPPORTED"
    );
    assert!(
        first.0.try_wait().unwrap().is_none(),
        "the first WM must keep running"
    );
}
