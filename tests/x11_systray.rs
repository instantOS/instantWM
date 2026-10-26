//! Mixed XEmbed/StatusNotifier tray regression on private, disjoint Xinerama
//! monitors. Run with `cargo test --test x11_systray -- --ignored --nocapture`.
//! Requires Xvfb, Xephyr and dbus-daemon; does not touch the running desktop.
use std::io::{BufRead, BufReader};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
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
fn server(directory: &Path, name: &str, command: &mut Command) -> (Process, String) {
    let file = directory.join(name);
    let fd = std::fs::File::create(&file).unwrap();
    unsafe {
        libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, 0);
    }
    let process = Process(
        command
            .args([
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
struct Item;
#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![(16, 16, [255, 250, 20, 50].repeat(16 * 16))]
    }
}

#[test]
#[ignore = "requires private Xvfb/Xephyr and dbus-daemon servers"]
fn mixed_tray_monitor_switch_and_pinning() {
    check_tray_on_monitors((800, 0));
}

#[test]
#[ignore = "requires private Xvfb/Xephyr and dbus-daemon servers"]
fn stacked_monitors_hide_and_restore_bars_and_tray() {
    check_tray_on_monitors((0, 600));
}

fn check_tray_on_monitors(second_origin: (i16, i16)) {
    let (second_x, second_y) = second_origin;
    let vertical = second_y != 0;
    let second_origin_arg = format!("{second_x},{second_y}");
    let dir = tempfile::tempdir().unwrap();
    let (_host, host_display) = server(
        dir.path(),
        "host-display",
        Command::new("Xvfb").args([
            "-screen",
            "0",
            if vertical {
                "800x1200x24"
            } else {
                "1600x600x24"
            },
        ]),
    );
    let (_server, display) = server(
        dir.path(),
        "display",
        Command::new("Xephyr").env("DISPLAY", host_display).args([
            "-origin",
            "0,0",
            "-screen",
            "800x600",
            "-origin",
            &second_origin_arg,
            "-screen",
            "800x600",
            "+xinerama",
            "-extension",
            "RANDR",
            "-no-host-grab",
        ]),
    );
    let mut bus = Process(
        Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut address = String::new();
    BufReader::new(bus.0.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    let address = address.trim();
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let atom = |name: &str| {
        conn.intern_atom(false, name.as_bytes())
            .unwrap()
            .reply()
            .unwrap()
            .atom
    };
    let selection = atom("_NET_SYSTEM_TRAY_S0");
    let opcode = atom("_NET_SYSTEM_TRAY_OPCODE");
    let info = atom("_XEMBED_INFO");
    let config = dir.path().join("config/instantwm");
    std::fs::create_dir_all(&config).unwrap();

    for pinning in [0, if vertical { 2 } else { 1 }] {
        std::fs::write(
            config.join("config.toml"),
            format!("[systray]\npinning = {pinning}\n"),
        )
        .unwrap();
        let socket = dir.path().join(format!("wm-{pinning}.sock"));
        let log = std::fs::File::create(dir.path().join(format!("wm-{pinning}.log"))).unwrap();
        let wm = Process(
            Command::new(env!("CARGO_BIN_EXE_instantwm"))
                .args(["--backend", "x11"])
                .env("DISPLAY", &display)
                .env("DBUS_SESSION_BUS_ADDRESS", address)
                .env("XDG_CONFIG_HOME", dir.path().join("config"))
                .env("INSTANTWM_AUTOSTART", "0")
                .env("INSTANTWM_LOG", "warn")
                .env("INSTANTWM_SOCKET_BIND", &socket)
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .unwrap(),
        );
        let ctl = |args: &[&str]| {
            Command::new(env!("CARGO_BIN_EXE_instantwmctl"))
                .env("INSTANTWM_SOCKET", &socket)
                .args(args)
                .output()
                .unwrap()
        };
        wait("WM IPC", || ctl(&["status"]).status.success().then_some(()));
        let monitors: serde_json::Value =
            serde_json::from_slice(&ctl(&["--json", "monitor", "list"]).stdout).unwrap();
        assert_eq!(
            monitors.as_array().unwrap().len(),
            2,
            "need disjoint monitors: {monitors}"
        );
        assert_eq!(monitors[1]["x"], second_x);
        assert_eq!(monitors[1]["y"], second_y);
        let manager = wait("tray manager", || {
            let win = conn
                .get_selection_owner(selection)
                .ok()?
                .reply()
                .ok()?
                .owner;
            (win != 0).then_some(win)
        });
        let mut icons = Vec::new();
        for _ in 0..6 {
            let win = conn.generate_id().unwrap();
            conn.create_window(
                x11rb::COPY_FROM_PARENT as u8,
                win,
                root,
                0,
                0,
                16,
                16,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new().background_pixel(0x20ee40),
            )
            .unwrap()
            .check()
            .unwrap();
            conn.change_property32(PropMode::REPLACE, win, info, info, &[0, 1])
                .unwrap();
            conn.send_event(
                false,
                manager,
                EventMask::NO_EVENT,
                ClientMessageEvent::new(32, manager, opcode, [0, 0, win, 0, 0]),
            )
            .unwrap();
            icons.push(win);
        }
        conn.flush().unwrap();
        wait("all six legacy icons docked", || {
            let children = conn.query_tree(manager).ok()?.reply().ok()?.children;
            icons.iter().all(|win| children.contains(win)).then_some(())
        });
        let item = zbus::blocking::connection::Builder::address(address)
            .unwrap()
            .serve_at("/StatusNotifierItem", Item)
            .unwrap()
            .build()
            .unwrap();
        let watcher = wait("StatusNotifierWatcher", || {
            zbus::blocking::Proxy::new(
                &item,
                "org.kde.StatusNotifierWatcher",
                "/StatusNotifierWatcher",
                "org.kde.StatusNotifierWatcher",
            )
            .ok()
            .filter(|proxy| proxy.get_property::<i32>("ProtocolVersion").is_ok())
        });
        watcher
            .call::<_, _, ()>("RegisterStatusNotifierItem", &("/StatusNotifierItem",))
            .unwrap();
        for target in [0usize, 1, 0, 1] {
            let name = monitors[target]["name"].as_str().unwrap();
            assert!(ctl(&["monitor", "switch", name]).status.success());
            let host = if pinning == 0 { target } else { pinning - 1 };
            let host_x = host as i16 * second_x;
            let host_y = host as i16 * second_y;
            wait("legacy tray follows the intended monitor", || {
                let geo = conn.get_geometry(manager).ok()?.reply().ok()?;
                (geo.x + geo.width as i16 == host_x + 800 && geo.y == host_y).then_some(())
            });
            println!("pinning={pinning}, selected monitor={target}, tray monitor={host}");
            let geo = conn.get_geometry(manager).unwrap().reply().unwrap();
            let sample_x = geo.x - (geo.height / 2) as i16;
            // SNI is painted immediately left of the native strip. Check its
            // actual pixels, not just a model that claims the icon is present.
            wait("SNI pixels adjacent to legacy icons", || {
                let image = conn
                    .get_image(
                        ImageFormat::Z_PIXMAP,
                        root,
                        sample_x,
                        host_y + (geo.height / 2) as i16,
                        1,
                        1,
                        u32::MAX,
                    )
                    .ok()?
                    .reply()
                    .ok()?;
                (image.data.get(..3) == Some(&[50, 20, 250])).then_some(())
            });
            for icon in &icons {
                assert_eq!(
                    conn.get_window_attributes(*icon)
                        .unwrap()
                        .reply()
                        .unwrap()
                        .map_state,
                    MapState::VIEWABLE,
                    "every mapped legacy icon must be visible"
                );
            }
            let other_x = (1 - host) as i16 * second_x;
            let other_y = (1 - host) as i16 * second_y;
            let old_icon_x = other_x + 800 - geo.width as i16 - (geo.height / 2) as i16;
            wait("old tray location has no stale SNI icon", || {
                let old_icon = conn
                    .get_image(
                        ImageFormat::Z_PIXMAP,
                        root,
                        old_icon_x,
                        other_y + (geo.height / 2) as i16,
                        1,
                        1,
                        u32::MAX,
                    )
                    .ok()?
                    .reply()
                    .ok()?;
                (old_icon.data.get(..3) != Some(&[50, 20, 250][..])).then_some(())
            });
            let bars = conn.query_tree(root).unwrap().reply().unwrap().children;
            assert!(
                bars.iter().any(|win| {
                    let geo = conn.get_geometry(*win).unwrap().reply().unwrap();
                    geo.x == other_x
                        && geo.y == other_y
                        && geo.width == 800
                        && geo.height > 1
                        && geo.height < 100
                }),
                "the other bar must regain its full width"
            );
        }
        if vertical {
            check_stacked_bar_visibility(&conn, root, manager, selection, &icons, &ctl);
        }
        let full_width = conn.get_geometry(manager).unwrap().reply().unwrap().width;
        conn.change_property32(PropMode::REPLACE, icons[0], info, info, &[0, 0])
            .unwrap();
        conn.flush().unwrap();
        wait("hidden icon releases its slot", || {
            (conn.get_geometry(manager).ok()?.reply().ok()?.width < full_width).then_some(())
        });
        conn.change_property32(PropMode::REPLACE, icons[0], info, info, &[0, 1])
            .unwrap();
        conn.flush().unwrap();
        wait("mapped icon regains its slot", || {
            (conn.get_geometry(manager).ok()?.reply().ok()?.width == full_width).then_some(())
        });
        let orientation = conn
            .get_property(
                false,
                manager,
                atom("_NET_SYSTEM_TRAY_ORIENTATION"),
                AtomEnum::CARDINAL,
                0,
                1,
            )
            .unwrap()
            .reply()
            .unwrap();
        assert_eq!(
            orientation.value32().and_then(|mut words| words.next()),
            Some(0)
        );
        for win in icons {
            conn.destroy_window(win).unwrap();
        }
        conn.flush().unwrap();
        drop(watcher);
        item.close().unwrap();
        drop(wm);
        wait("old tray owner released", || {
            (conn
                .get_selection_owner(selection)
                .ok()?
                .reply()
                .ok()?
                .owner
                == 0)
                .then_some(())
        });
    }
}

/// Observe server mapping, not just coordinates: hiding must work even when
/// another output occupies the pixels above or below the owning monitor.
fn check_stacked_bar_visibility(
    conn: &x11rb::rust_connection::RustConnection,
    root: Window,
    manager: Window,
    selection: Atom,
    icons: &[Window],
    ctl: &impl Fn(&[&str]) -> std::process::Output,
) {
    let run = |args: &[&str]| {
        let output = ctl(args);
        assert!(output.status.success(), "{args:?}: {output:?}");
    };
    let map_state = |win| {
        conn.get_window_attributes(win)
            .unwrap()
            .reply()
            .unwrap()
            .map_state
    };
    let geometry = |win| conn.get_geometry(win).unwrap().reply().unwrap();
    // The switching exercise ends on the lower output. Its top bar and the
    // upper output's bottom strip are the two windows that used to spill.
    let windows = conn.query_tree(root).unwrap().reply().unwrap().children;
    let top = *windows
        .iter()
        .find(|win| {
            let geo = geometry(**win);
            **win != manager
                && geo.x == 0
                && geo.y == 600
                && geo.width > 400
                && geo.height > 1
                && geo.height < 100
        })
        .expect("lower top bar");
    run(&["action", "toggle_bottom_bar", "on"]);
    let bottom = wait("upper bottom strip", || {
        windows.iter().copied().find(|win| {
            let geo = geometry(*win);
            geo.x == 0
                && geo.y > 500
                && geo.y < 600
                && geo.width == 800
                && map_state(*win) == MapState::VIEWABLE
        })
    });
    let assert_owner = || {
        assert_eq!(
            conn.get_selection_owner(selection)
                .unwrap()
                .reply()
                .unwrap()
                .owner,
            manager
        );
        let children = conn.query_tree(manager).unwrap().reply().unwrap().children;
        assert!(
            icons.iter().all(|win| children.contains(win)),
            "docked clients must survive hiding"
        );
    };
    for _ in 0..2 {
        run(&["action", "toggle_bar"]);
        run(&["action", "toggle_bottom_bar", "off"]);
        wait("hidden bars and tray unmapped", || {
            [top, bottom, manager]
                .iter()
                .all(|win| map_state(*win) == MapState::UNMAPPED)
                .then_some(())
        });
        assert_owner();
        for icon in icons {
            assert_eq!(map_state(*icon), MapState::UNVIEWABLE);
        }
        run(&["action", "toggle_bar"]);
        run(&["action", "toggle_bottom_bar", "on"]);
        wait("bars and tray restored", || {
            [top, bottom, manager]
                .iter()
                .all(|win| map_state(*win) == MapState::VIEWABLE)
                .then_some(())
        });
        assert_eq!(geometry(manager).y, 600);
        assert_eq!(geometry(top).y, 600);
        assert_owner();
        for icon in icons {
            assert_eq!(map_state(*icon), MapState::VIEWABLE);
        }
    }

    // Fullscreen uses the same visibility policy, including the bottom strip.
    let lower_bottom = *windows
        .iter()
        .find(|win| {
            let geo = geometry(**win);
            geo.x == 0 && geo.y > 1100 && geo.y < 1200 && geo.width == 800
        })
        .expect("lower bottom strip");
    let atom = |name: &[u8]| conn.intern_atom(false, name).unwrap().reply().unwrap().atom;
    let state = atom(b"_NET_WM_STATE");
    let fullscreen = atom(b"_NET_WM_STATE_FULLSCREEN");
    let client = conn.generate_id().unwrap();
    conn.create_window(
        x11rb::COPY_FROM_PARENT as u8,
        client,
        root,
        100,
        700,
        200,
        100,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new(),
    )
    .unwrap();
    conn.change_property32(
        PropMode::REPLACE,
        client,
        state,
        AtomEnum::ATOM,
        &[fullscreen],
    )
    .unwrap();
    conn.map_window(client).unwrap();
    conn.flush().unwrap();
    wait("fullscreen hides owning monitor bars and tray", || {
        [top, lower_bottom, manager]
            .iter()
            .all(|win| map_state(*win) == MapState::UNMAPPED)
            .then_some(())
    });
    assert_eq!(map_state(bottom), MapState::VIEWABLE);
    assert_owner();
    conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        ClientMessageEvent::new(32, client, state, [0, fullscreen, 0, 1, 0]),
    )
    .unwrap();
    conn.flush().unwrap();
    wait("leaving fullscreen restores bars and tray", || {
        [top, lower_bottom, manager]
            .iter()
            .all(|win| map_state(*win) == MapState::VIEWABLE)
            .then_some(())
    });
    assert_owner();
    for icon in icons {
        assert_eq!(map_state(*icon), MapState::VIEWABLE);
    }
    conn.destroy_window(client).unwrap();
    conn.flush().unwrap();
}
