//! End-to-end checks for X11 client state, passive grabs, and stacking.
//! Run with `cargo test --test x11_client_state -- --ignored --nocapture`.
//! Requires Xvfb; uses a private display and does not touch the running desktop.
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::ErrorKind;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

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
        .args(["--backend", "x11"])
        .env("DISPLAY", display)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("INSTANTWM_AUTOSTART", "0")
        .env("INSTANTWM_SOCKET_BIND", socket)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn create_window(conn: &RustConnection, root: Window) -> Window {
    let window = conn.generate_id().unwrap();
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        window,
        root,
        10,
        100,
        200,
        150,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new(),
    )
    .unwrap()
    .check()
    .unwrap();
    window
}

fn grab(conn: &RustConnection, root: Window, code: u8) -> Result<(), ReplyError> {
    conn.grab_key(
        false,
        root,
        ModMask::default(),
        code,
        GrabMode::ASYNC,
        GrabMode::ASYNC,
    )?
    .check()
}

fn grabbed(conn: &RustConnection, root: Window, code: u8) -> bool {
    match grab(conn, root, code) {
        Err(ReplyError::X11Error(error)) if error.error_kind == ErrorKind::Access => true,
        Ok(()) => {
            conn.ungrab_key(code, root, ModMask::default())
                .unwrap()
                .check()
                .unwrap();
            false
        }
        Err(error) => panic!("probing passive grab failed: {error}"),
    }
}

#[test]
#[ignore = "requires a private Xvfb server"]
fn focus_scope_restores_desktop_grabs_and_retries_contested_wm_grabs() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let config_dir = home.join(".config/instantwm");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        r#"
[[keybinds]]
key = "F10"
action = "focus_next"

[[keybinds]]
key = "F11"
action = "focus_next"

[[desktop_keybinds]]
key = "F12"
action = "focus_next"
"#,
    )
    .unwrap();
    let (_server, display) = server(dir.path());
    let (conn, screen_num) = RustConnection::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen_num].root;
    let start = conn.setup().min_keycode;
    let count = conn.setup().max_keycode - start + 1;
    let mapping = conn
        .get_keyboard_mapping(start, count)
        .unwrap()
        .reply()
        .unwrap();
    let keycode = |keysym| {
        start
            + mapping
                .keysyms
                .chunks(usize::from(mapping.keysyms_per_keycode))
                .position(|codes| codes[0] == keysym)
                .unwrap() as u8
    };
    let [f10, f11, f12] = [0xffc7, 0xffc8, 0xffc9].map(keycode);
    grab(&conn, root, f10).unwrap();
    let active = conn
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .unwrap()
        .reply()
        .unwrap()
        .atom;

    let socket = dir.path().join("wm.sock");
    let _wm = Process(
        wm_command(env!("CARGO_BIN_EXE_instantwm"), &display, &home, &socket)
            .spawn()
            .unwrap(),
    );
    wait("WM IPC startup", || socket.exists().then_some(()));
    assert!(grabbed(&conn, root, f11));
    assert!(grabbed(&conn, root, f12));
    conn.ungrab_key(f10, root, ModMask::default())
        .unwrap()
        .check()
        .unwrap();

    let window = create_window(&conn, root);
    conn.map_window(window).unwrap().check().unwrap();
    // X11 applies passive grabs before advertising the new active window.
    wait("client focus", || {
        property_words(&conn, root, active, AtomEnum::WINDOW, 1)
            .filter(|value| value.as_slice() == [window])
    });
    assert!(!grabbed(&conn, root, f12), "desktop grab is inactive");
    assert!(
        grabbed(&conn, root, f10),
        "contested global grab was retried"
    );
    conn.destroy_window(window).unwrap().check().unwrap();
    wait("desktop focus", || {
        property_words(&conn, root, active, AtomEnum::WINDOW, 1)
            .is_none()
            .then_some(())
    });
    assert!(grabbed(&conn, root, f12), "desktop grab was restored");
    assert!(grabbed(&conn, root, f11));
    assert!(grabbed(&conn, root, f10));
}

#[test]
#[ignore = "requires a private Xvfb server"]
fn fullscreen_parent_keeps_its_transient_above_it() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let (_server, display) = server(dir.path());
    let (conn, screen_num) = RustConnection::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen_num].root;
    let atom = |name: &[u8]| conn.intern_atom(false, name).unwrap().reply().unwrap().atom;
    let wm_state = atom(b"_NET_WM_STATE");
    let client_info = atom(b"_NET_CLIENT_INFO");
    let fullscreen = atom(b"_NET_WM_STATE_FULLSCREEN");
    let window_type = atom(b"_NET_WM_WINDOW_TYPE");
    let dialog_type = atom(b"_NET_WM_WINDOW_TYPE_DIALOG");
    let socket = dir.path().join("wm.sock");
    let _wm = Process(
        wm_command(env!("CARGO_BIN_EXE_instantwm"), &display, &home, &socket)
            .spawn()
            .unwrap(),
    );
    wait("WM IPC startup", || socket.exists().then_some(()));

    let parent = create_window(&conn, root);
    let dialog = create_window(&conn, root);
    conn.change_property32(
        PropMode::REPLACE,
        dialog,
        AtomEnum::WM_TRANSIENT_FOR,
        AtomEnum::WINDOW,
        &[parent],
    )
    .unwrap();
    for window in [parent, dialog] {
        conn.change_property32(
            PropMode::REPLACE,
            window,
            window_type,
            AtomEnum::ATOM,
            &[dialog_type],
        )
        .unwrap();
        conn.map_window(window).unwrap().check().unwrap();
        wait("client adopted", || {
            property_words(&conn, window, client_info, AtomEnum::CARDINAL, 2)
        });
    }

    conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        ClientMessageEvent::new(32, parent, wm_state, [1, fullscreen, 0, 1, 0]),
    )
    .unwrap()
    .check()
    .unwrap();
    wait("parent enters fullscreen", || {
        property_words(&conn, parent, wm_state, AtomEnum::ATOM, 8)
            .filter(|states| states.contains(&fullscreen))
    });
    // IPC runs after queued X11 events, including the fullscreen restack.
    assert!(
        Command::new(env!("CARGO_BIN_EXE_instantwmctl"))
            .arg("status")
            .env("INSTANTWM_SOCKET", &socket)
            .output()
            .unwrap()
            .status
            .success()
    );
    let stack = conn.query_tree(root).unwrap().reply().unwrap().children;
    let parent_index = stack.iter().position(|window| *window == parent).unwrap();
    let dialog_index = stack.iter().position(|window| *window == dialog).unwrap();
    assert!(
        parent_index < dialog_index,
        "fullscreen parent covered its dialog"
    );
}

fn property_words(
    conn: &RustConnection,
    window: Window,
    property: Atom,
    type_: impl Into<Atom>,
    length: u32,
) -> Option<Vec<u32>> {
    let reply = conn
        .get_property(false, window, property, type_, 0, length)
        .ok()?
        .reply()
        .ok()?;
    if reply.format != 32 {
        return None;
    }
    Some(reply.value32()?.collect())
}

#[test]
#[ignore = "requires a private Xvfb server"]
fn restart_restores_full_tag_mask_over_the_ewmh_desktop_mirror() {
    let exe = env!("CARGO_BIN_EXE_instantwm");
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let (_server, display) = server(dir.path());

    let (conn, screen_num) = RustConnection::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen_num].root;
    let atom = |name: &[u8]| conn.intern_atom(false, name).unwrap().reply().unwrap().atom;
    let client_info = atom(b"_NET_CLIENT_INFO");
    let wm_desktop = atom(b"_NET_WM_DESKTOP");
    let wm_check = atom(b"_NET_SUPPORTING_WM_CHECK");

    let first = Process(
        wm_command(&exe, &display, &home, &dir.path().join("wm1.sock"))
            .spawn()
            .unwrap(),
    );
    wait("first WM advertises _NET_SUPPORTING_WM_CHECK", || {
        property_words(&conn, root, wm_check, AtomEnum::WINDOW, 1)
    });

    let window = conn.generate_id().unwrap();
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        window,
        root,
        10,
        10,
        200,
        150,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new(),
    )
    .unwrap();
    conn.map_window(window).unwrap();
    conn.flush().unwrap();

    // The WM must publish the property in the format its reader expects.
    let values = wait("first WM publishes _NET_CLIENT_INFO", || {
        property_words(&conn, window, client_info, AtomEnum::CARDINAL, 2)
    });
    assert_eq!(values.len(), 2);

    // Simulate the previous session's properties. The desktop hint names tag 2,
    // while the richer property names tags 1 and 3; the latter must win.
    const TAGS_1_AND_3: u32 = (1 << 0) | (1 << 2);
    drop(first);
    conn.change_property32(
        PropMode::REPLACE,
        window,
        client_info,
        AtomEnum::CARDINAL,
        &[TAGS_1_AND_3, 0],
    )
    .unwrap();
    conn.change_property32(
        PropMode::REPLACE,
        window,
        wm_desktop,
        AtomEnum::CARDINAL,
        &[1],
    )
    .unwrap();
    conn.flush().unwrap();

    let second = Process(
        wm_command(&exe, &display, &home, &dir.path().join("wm2.sock"))
            .spawn()
            .unwrap(),
    );
    // The next WM rewrites the desktop mirror from the restored tags, proving
    // that this window has been re-managed.
    let desktop = wait("second WM re-manages the client", || {
        property_words(&conn, window, wm_desktop, AtomEnum::CARDINAL, 1)
            .filter(|values| values.as_slice() == [0])
    });
    assert_eq!(desktop, [0]);

    let values = property_words(&conn, window, client_info, AtomEnum::CARDINAL, 2).unwrap();
    assert_eq!(values, [TAGS_1_AND_3, 0]);

    drop(second);
}
