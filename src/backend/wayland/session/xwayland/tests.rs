use super::*;
use smithay::reexports::calloop::EventLoop;
use std::sync::mpsc;
use x11rb::{
    connection::Connection,
    properties::{WmSizeHints, WmSizeHintsSpecification},
    protocol::xproto::{ConfigureWindowAux, ConnectionExt, CreateWindowAux, PropMode, WindowClass},
    wrapper::ConnectionExt as _,
};

type Loop = EventLoop<'static, WaylandState>;

fn drive_until(
    event_loop: &mut Loop,
    state: &mut WaylandState,
    timeout: Duration,
    mut done: impl FnMut(&WaylandState) -> bool,
) {
    let deadline = Instant::now() + timeout;
    while !done(state) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for Xwayland lifecycle transition"
        );
        event_loop
            .dispatch(Some(Duration::from_millis(10)), state)
            .unwrap();
        state.native.display_handle.flush_clients().unwrap();
        crate::backend::wayland::runtime::dispatch::drain_command_queue(state);
    }
}

fn pending_connection(
    display: &str,
) -> mpsc::Receiver<Result<x11rb::rust_connection::RustConnection, x11rb::errors::ConnectError>> {
    let display = display.to_owned();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = x11rb::connect(Some(&display)).map(|(conn, _)| conn);
        let _ = tx.send(result);
    });
    rx
}

fn finish_connect(
    event_loop: &mut Loop,
    state: &mut WaylandState,
    rx: mpsc::Receiver<Result<x11rb::rust_connection::RustConnection, x11rb::errors::ConnectError>>,
) -> x11rb::rust_connection::RustConnection {
    let mut result = None;
    drive_until(event_loop, state, Duration::from_secs(10), |_| {
        result = rx.try_recv().ok();
        result.is_some()
    });
    result.unwrap().unwrap()
}

fn connect(
    event_loop: &mut Loop,
    state: &mut WaylandState,
    display: &str,
) -> x11rb::rust_connection::RustConnection {
    finish_connect(event_loop, state, pending_connection(display))
}

#[test]
fn reserving_a_display_starts_no_server_and_drop_releases_the_address() {
    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    std::fs::create_dir_all("/tmp/.X11-unix").unwrap();
    let display = XWaylandDisplay::new(None, true).unwrap();
    let number = display.display_number();
    XWaylandSession::reserve(&mut state, display).unwrap();
    for _ in 0..5 {
        event_loop
            .dispatch(Some(Duration::ZERO), &mut state)
            .unwrap();
    }
    assert!(
        state
            .native
            .xwayland_session
            .as_ref()
            .unwrap()
            .server
            .is_none()
    );
    assert!(state.native.xwm.is_none());
    assert_eq!(state.native.xdisplay, Some(number));
    drop(state.native.xwayland_session.take());
    assert!(XWaylandDisplay::new(Some(number), true).is_ok());
}

#[test]
#[ignore = "requires Xwayland; runs an isolated headless server"]
fn floating_window_honors_client_position_and_decoration_requests() {
    use crate::types::{ClientPlacement, Rect, Size};

    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    state
        .native
        .create_output("TEST", Size::new(1920, 1080), None);
    state.wm.core.state.model.monitors.push(
        crate::test_support::MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1920, 1080))
            .build(),
    );
    std::fs::create_dir_all("/tmp/.X11-unix").unwrap();
    let display = XWaylandDisplay::new(None, true).unwrap();
    let address = format!(":{}", display.display_number());
    XWaylandSession::reserve(&mut state, display).unwrap();
    let conn = connect(&mut event_loop, &mut state, &address);
    let root = conn.setup().roots[0].root;
    let window = conn.generate_id().unwrap();
    let motif = conn
        .intern_atom(false, b"_MOTIF_WM_HINTS")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    conn.create_window(
        0,
        window,
        root,
        0,
        33,
        420,
        160,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new(),
    )
    .unwrap();
    WmSizeHints {
        position: Some((WmSizeHintsSpecification::UserSpecified, 0, 33)),
        min_size: Some((420, 160)),
        max_size: Some((420, 160)),
        ..WmSizeHints::new()
    }
    .set_normal_hints(&conn, window)
    .unwrap();
    let decoration_flag = crate::backend::x11::constants::MWM_HINTS_DECORATIONS;
    conn.change_property32(
        PropMode::REPLACE,
        window,
        motif,
        motif,
        &[decoration_flag, 0, 0, 0, 0],
    )
    .unwrap();
    conn.map_window(window).unwrap();
    conn.flush().unwrap();

    drive_until(&mut event_loop, &mut state, Duration::from_secs(5), |s| {
        s.native
            .window_id_for_x11_window(window)
            .is_some_and(|win| s.wm.core.state.model.client(win).is_some())
    });
    let win = state.native.window_id_for_x11_window(window).unwrap();
    let client = state.wm.core.state.model.client(win).unwrap();
    assert_eq!(client.placement(), ClientPlacement::Floating);
    assert_eq!(client.border_width, 0);
    assert_eq!(client.old_border_width, 0);

    let target = Rect::new(1162, 92, 420, 160);
    conn.configure_window(window, &ConfigureWindowAux::new().x(target.x).y(target.y))
        .unwrap();
    conn.flush().unwrap();
    drive_until(&mut event_loop, &mut state, Duration::from_secs(5), |s| {
        s.wm.core
            .state
            .model
            .client(win)
            .is_some_and(|client| client.geo == target)
    });
    let actual = conn
        .translate_coordinates(window, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    assert_eq!(
        (i32::from(actual.dst_x), i32::from(actual.dst_y)),
        (target.x, target.y)
    );

    let decorations = crate::backend::x11::constants::MWM_DECOR_ALL;
    conn.change_property32(
        PropMode::REPLACE,
        window,
        motif,
        motif,
        &[decoration_flag, 0, decorations, 0, 0],
    )
    .unwrap();
    conn.flush().unwrap();
    drive_until(&mut event_loop, &mut state, Duration::from_secs(5), |s| {
        s.wm.core.state.model.client(win).is_some_and(|client| {
            client.border_width == s.wm.core.state.config.window.border_width_px
        })
    });
}

#[test]
#[ignore = "requires Xwayland; runs an isolated headless server"]
fn lazy_start_idle_shutdown_and_crash_recovery_keep_the_same_display() {
    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    state
        .native
        .create_output("TEST", crate::types::Size::new(1024, 768), None);
    std::fs::create_dir_all("/tmp/.X11-unix").unwrap();
    let display = XWaylandDisplay::new(None, true).unwrap();
    let number = display.display_number();
    XWaylandSession::reserve(&mut state, display).unwrap();
    let address = format!(":{number}");
    smithay::wayland::selection::data_device::set_data_device_selection(
        &state.native.display_handle,
        &state.native.seat,
        vec!["text/plain".into()],
        (),
    );
    let first = pending_connection(&address);
    let second = pending_connection(&address);
    let first = finish_connect(&mut event_loop, &mut state, first);
    let second = finish_connect(&mut event_loop, &mut state, second);
    drive_until(&mut event_loop, &mut state, Duration::from_secs(5), |s| {
        s.native.xwm.is_some()
    });
    // A selection that existed before startup must be offered to X11 clients.
    let clipboard = first
        .intern_atom(false, b"CLIPBOARD")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    drive_until(&mut event_loop, &mut state, Duration::from_secs(5), |_| {
        first
            .get_selection_owner(clipboard)
            .unwrap()
            .reply()
            .unwrap()
            .owner
            != x11rb::NONE
    });
    assert_eq!(
        state.native.xwayland_session.as_ref().unwrap().generation,
        1
    );
    assert!(state.native.xwm.is_some());
    // A client without windows must keep Xwayland alive beyond its idle timeout.
    drive_until(&mut event_loop, &mut state, Duration::from_secs(15), |s| {
        s.native
            .xwayland_session
            .as_ref()
            .unwrap()
            .server
            .as_ref()
            .unwrap()
            .started
            .elapsed()
            > Duration::from_secs(11)
    });
    assert!(state.native.xwm.is_some());
    drop(first);
    drop(second);
    drive_until(&mut event_loop, &mut state, Duration::from_secs(15), |s| {
        s.native.xwayland_session.as_ref().unwrap().server.is_none()
    });
    assert_eq!(state.native.xdisplay, Some(number));
    let client = connect(&mut event_loop, &mut state, &address);
    let generation = state.native.xwayland_session.as_ref().unwrap().generation;
    assert_eq!(generation, 2);
    // Verify an override-redirect launcher maps and is removed on server death.
    let root = client.setup().roots[0].root;
    let window = client.generate_id().unwrap();
    client
        .create_window(
            0,
            window,
            root,
            0,
            0,
            500,
            30,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new().override_redirect(1),
        )
        .unwrap();
    client.map_window(window).unwrap();
    client.flush().unwrap();
    drive_until(&mut event_loop, &mut state, Duration::from_secs(5), |s| {
        s.native.space.elements().any(|w| w.x11_surface().is_some())
    });
    let pid = state
        .native
        .xwayland_session
        .as_ref()
        .unwrap()
        .server
        .as_ref()
        .unwrap()
        .pid;
    // This PID comes from Child::id(), not SO_PEERCRED on the inherited socket.
    assert!(
        rustix::process::kill_process(
            rustix::process::Pid::from_raw(i32::try_from(pid).unwrap()).unwrap(),
            rustix::process::Signal::KILL,
        )
        .is_ok()
    );
    drive_until(&mut event_loop, &mut state, Duration::from_secs(5), |s| {
        s.native.xwayland_session.as_ref().unwrap().server.is_none()
    });
    assert!(
        !state
            .native
            .space
            .elements()
            .any(|w| w.x11_surface().is_some())
    );
    assert!(state.native.xwm.is_none());
    assert_eq!(state.native.xdisplay, Some(number));
    drop(client);
    let replacement = connect(&mut event_loop, &mut state, &address);
    assert_eq!(
        state.native.xwayland_session.as_ref().unwrap().generation,
        3
    );
    // A delayed callback from the old process cannot tear down the replacement.
    stop(&mut state, generation, true);
    assert!(state.native.xwm.is_some());
    drop(replacement);
}
