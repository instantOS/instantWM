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

    // A position-only slide must also move the X input window when it lands.
    let element = state.native.find_window(win).unwrap().clone();
    state
        .native
        .space
        .map_element(element, (moved.x, moved.y), false);
    let shown = Rect::new(20, 0, initial.w, initial.h);
    state.wm.core.state.config.animations.enabled = true;
    state.wm.core.state.model.sync_client_geometry(win, shown);
    state.native.set_window_target_rect(
        &state.wm.core.state,
        win,
        shown,
        0,
        super::WindowMoveMode::AnimateFrom {
            from: moved,
            duration: std::time::Duration::ZERO,
        },
    );
    state.native.tick_animations(&state.wm.core.state);
    assert!(!state.native.has_active_window_animations());
    let actual = conn.get_geometry(xwindow).unwrap().reply().unwrap();
    assert_eq!((actual.x, actual.y), (20, 0));

    // Model a completed growth with an outstanding old buffer. There is no
    // Wayland surface in this fixture, so X11Surface's initial geometry stands
    // in for the old buffer until we configure the new geometry below.
    let grown = Rect::new(shown.x, shown.y, shown.w + 200, shown.h);
    let started_at = std::time::Instant::now();
    let mut animation = super::WaylandWindowAnimation::new(
        crate::animation::WindowAnimation {
            from: shown,
            to: grown,
            started_at,
            duration: std::time::Duration::ZERO,
        },
        (shown.w, shown.h).into(),
        true,
        0,
        0,
    );
    animation.tick(started_at, (shown.w, shown.h).into());
    state.native.window_animations.insert(win, animation);
    state
        .native
        .geometry_sync
        .get_mut(&win)
        .unwrap()
        .sent(grown.size(), None);
    state
        .native
        .reconcile_completed_window_animation(win, (shown.w, shown.h).into());
    assert!(state.native.animation_targets_outer_rect(win, grown));
    assert!(!state.native.has_active_window_animations());
    let element = state.native.find_window(win).unwrap();
    assert_eq!(
        state.native.space.element_location(element),
        Some((grown.x + 200, grown.y).into())
    );

    element
        .x11_surface()
        .unwrap()
        .configure(Rectangle::new(
            (grown.x, grown.y).into(),
            (grown.w, grown.h).into(),
        ))
        .unwrap();
    state
        .native
        .reconcile_completed_window_animation(win, (grown.w, grown.h).into());
    assert!(!state.native.animation_targets_outer_rect(win, grown));
    let element = state.native.find_window(win).unwrap();
    assert_eq!(
        state.native.space.element_location(element),
        Some((grown.x, grown.y).into())
    );
}
