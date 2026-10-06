//! Exercise Smithay's X11Surface configure path against an isolated X server.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, atomic::AtomicBool};

use smithay::desktop::Window;
use smithay::utils::Rectangle;
use smithay::xwayland::{X11Surface, xwm::Atoms};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, CreateWindowAux, WindowClass};

use crate::test_support::{MonitorBuilder, add_client};
use crate::types::{Client, ClientMode, Rect, WindowId};

struct TestServer(Child);

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "requires Xvfb; run explicitly with --ignored"]
fn x11_position_only_snap_configures_the_supplied_origin_without_a_pending_resize() {
    let mut server = TestServer(
        Command::new("Xvfb")
            .args([
                "-displayfd",
                "1",
                "-screen",
                "0",
                "1920x1080x24",
                "-nolisten",
                "tcp",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start isolated X server"),
    );
    let mut display_number = String::new();
    BufReader::new(server.0.stdout.take().unwrap())
        .read_line(&mut display_number)
        .unwrap();
    let display = format!(":{}", display_number.trim());
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let conn = Arc::new(conn);
    let root = conn.setup().roots[screen].root;
    let xwindow = conn.generate_id().unwrap();
    conn.create_window(
        0,
        xwindow,
        root,
        100,
        100,
        640,
        480,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new(),
    )
    .unwrap()
    .check()
    .unwrap();
    let initial = Rect::new(100, 100, 640, 480);
    let surface = X11Surface::new(
        None,
        xwindow,
        false,
        Arc::downgrade(&conn),
        Atoms::new(&*conn).unwrap().reply().unwrap(),
        None,
        Rectangle::new((initial.x, initial.y).into(), (initial.w, initial.h).into()),
        Arc::new(AtomicBool::new(false)),
    );
    let element = Window::new_x11_window(surface);
    let (_event_loop, mut state) = crate::test_support::new_compositor();

    let monitor = state.wm.core.state.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1920, 1080))
            .build(),
    );
    let win = WindowId(91);
    add_client(
        &mut state.wm.core.state.model,
        monitor,
        Client {
            win,
            geo: initial,
            mode: ClientMode::floating(),
            ..Client::default()
        },
    );
    state.native.window_index.insert(win, element);
    state
        .native
        .resize_window(&state.wm.core.state, win, initial);
    assert_eq!(
        state
            .native
            .geometry_sync
            .get(&win)
            .unwrap()
            .scheduled_size(),
        None
    );

    // Deliberately leave model geometry at the old origin: the protocol
    // dispatcher must use its supplied rectangle, not re-read the model.
    let moved = Rect::new(400, 250, initial.w, initial.h);
    state.native.resize_window(&state.wm.core.state, win, moved);
    assert_eq!(
        state
            .native
            .geometry_sync
            .get(&win)
            .unwrap()
            .scheduled_size(),
        None
    );
    let actual = conn.get_geometry(xwindow).unwrap().reply().unwrap();
    assert_eq!(
        (actual.x, actual.y, actual.width, actual.height),
        (400, 250, 640, 480)
    );
}
