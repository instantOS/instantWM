use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zbus::zvariant::{OwnedValue, Value};

use crate::types::Size;

use super::{
    MenuAction, MenuToggle, NativeMenuRequest, StatusNotifierItem, StatusNotifierRuntime,
    StatusNotifierTray, StatusNotifierWorker, SystrayEventTx, WORKER_RETRY_MIN, WatcherMode,
    WatcherState, clear_native_menu_request, dbus_icon_bytes_to_rgba, handle_name_lost,
    handle_unregistered, id_matches_service, menu_entry_from_properties,
    select_largest_valid_pixmap, set_native_menu_request, strip_menu_mnemonics,
};

fn string_value(value: &str) -> OwnedValue {
    OwnedValue::try_from(Value::from(value)).expect("string is representable as an owned value")
}

#[test]
fn native_menu_request_handoff_records_and_clears_the_anchor() {
    let slot = Arc::new(Mutex::new(None));
    let anchor = crate::types::Point::new(1910, 16);

    set_native_menu_request(
        &slot,
        anchor,
        "org.example.Tray",
        "/StatusNotifierItem",
        Some(42),
    );
    assert_eq!(
        slot.lock().unwrap().as_ref().map(|request| request.anchor),
        Some(anchor)
    );

    clear_native_menu_request(Some(&slot));
    assert!(slot.lock().unwrap().is_none());
}

#[test]
fn native_menu_request_only_matches_its_dbus_owner() {
    let request = NativeMenuRequest {
        created: std::time::Instant::now(),
        anchor: crate::types::Point::new(10, 20),
        service: "org.example.Tray".to_string(),
        path: "/StatusNotifierItem".to_string(),
        owner_pid: Some(42),
    };

    assert!(request.matches_client_pid(Some(42)));
    assert!(!request.matches_client_pid(Some(43)));
    assert!(!request.matches_client_pid(None));

    let unresolved = NativeMenuRequest {
        owner_pid: None,
        ..request
    };
    assert!(!unresolved.matches_client_pid(Some(42)));
}

#[test]
fn dbus_icon_bytes_are_decoded_from_argb_to_rgba() {
    let bytes = [
        0xff, 0x00, 0x82, 0xc9, // opaque Nextcloud blue
        0x40, 0x11, 0x22, 0x33, // translucent pixel
    ];

    let rgba = dbus_icon_bytes_to_rgba(&bytes, Size::new(2, 1)).expect("valid icon bytes");

    assert_eq!(rgba, vec![0x00, 0x82, 0xc9, 0xff, 0x11, 0x22, 0x33, 0x40]);
}

#[test]
fn largest_valid_icon_pixmap_is_selected() {
    let selected = select_largest_valid_pixmap(vec![
        (16, 16, vec![0; 16 * 16 * 4]),
        (32, 32, vec![0; 32 * 32 * 4]),
        (64, 64, vec![0; 8]),
    ])
    .expect("a valid pixmap");

    assert_eq!(selected.0, Size::new(32, 32));
}

#[test]
fn hidden_and_empty_menu_entries_are_omitted() {
    let hidden = HashMap::from([
        ("label".to_string(), string_value("Hidden")),
        ("visible".to_string(), OwnedValue::from(false)),
    ]);
    let empty = HashMap::new();

    assert!(menu_entry_from_properties(1, &hidden, false).is_none());
    assert!(menu_entry_from_properties(2, &empty, false).is_none());
}

#[test]
fn separators_are_non_interactive() {
    let properties = HashMap::from([("type".to_string(), string_value("separator"))]);

    let entry = menu_entry_from_properties(3, &properties, false).expect("separator");

    assert!(entry.separator);
    assert!(!entry.enabled);
}

#[test]
fn submenu_and_toggle_properties_are_preserved() {
    let properties = HashMap::from([
        ("label".to_string(), string_value("_Notifications")),
        ("children-display".to_string(), string_value("submenu")),
        ("toggle-type".to_string(), string_value("checkmark")),
        ("toggle-state".to_string(), OwnedValue::from(1i32)),
    ]);

    let entry = menu_entry_from_properties(7, &properties, false).expect("menu entry");

    assert_eq!(entry.label, "Notifications");
    assert_eq!(entry.toggle, MenuToggle::Check(true));
    assert_eq!(entry.action, MenuAction::OpenSubmenu(7));
}

#[test]
fn menu_mnemonics_preserve_escaped_underscores() {
    assert_eq!(strip_menu_mnemonics("_Save __As"), "Save _As");
}

#[test]
fn stopped_worker_clears_stale_state_and_schedules_bounded_restart() {
    let (cmd_tx, cmd_rx) = channel();
    let (evt_tx, evt_rx) = channel();
    let thread = std::thread::spawn(move || {
        drop(cmd_rx);
        drop(evt_tx);
    });
    while !thread.is_finished() {
        std::thread::yield_now();
    }

    let mut runtime = StatusNotifierRuntime {
        worker: Some(StatusNotifierWorker {
            cmd_tx,
            evt_rx,
            thread,
        }),
        restart_at: None,
        retry_delay: WORKER_RETRY_MIN,
        next_menu_session_id: AtomicU64::new(1),
        native_menu_request: Some(Arc::new(Mutex::new(None))),
        wake: None,
    };
    let mut tray = StatusNotifierTray {
        items: vec![StatusNotifierItem {
            service: "org.example.Tray".to_string(),
            path: "/StatusNotifierItem".to_string(),
            icon_rgba: Arc::from(vec![0, 0, 0, 0]),
            icon_size: Size::new(1, 1),
        }],
    };
    let mut menu = crate::systray::TrayMenuState::default();
    menu.begin(4);
    menu.apply(4, Some(crate::systray::MenuView::default()));

    assert!(runtime.poll_events(&mut tray, &mut menu));
    assert!(runtime.worker.is_none());
    assert!(runtime.restart_at.is_some());
    assert_eq!(runtime.retry_delay, Duration::from_secs(2));
    assert!(tray.items.is_empty());
    assert!(menu.presentation().is_none());

    for _ in 0..10 {
        runtime.schedule_restart();
    }
    assert_eq!(runtime.retry_delay, Duration::from_secs(60));
}

#[test]
fn dbus_menu_raw_layout_parses_into_menu_entries() {
    let mut props = HashMap::new();
    props.insert("label".to_string(), string_value("Exit"));
    props.insert("enabled".to_string(), OwnedValue::from(true));
    props.insert("visible".to_string(), OwnedValue::from(true));

    let child_tuple = (7i32, props, Vec::<OwnedValue>::new());
    let child_val = OwnedValue::try_from(Value::from(child_tuple)).expect("valid tuple");

    let entry = super::menu::parse_menu_entry(child_val)
        .expect("parsed successfully")
        .expect("entry present");

    assert_eq!(entry.label, "Exit");
    assert_eq!(entry.action, MenuAction::Activate(7));
    assert!(entry.enabled);
    assert!(!entry.separator);
}

/// An event sender without a wake ping, for exercising the discovery handlers.
fn quiet_evt_tx() -> (SystrayEventTx, std::sync::mpsc::Receiver<super::SystrayEvt>) {
    let (tx, rx) = channel();
    (SystrayEventTx { tx, wake: None }, rx)
}

#[test]
fn item_ids_match_their_hosting_bus_name() {
    assert!(id_matches_service(":1.42/StatusNotifierItem", ":1.42"));
    assert!(id_matches_service(
        "org.kde.StatusNotifierItem-4242-1/StatusNotifierItem",
        "org.kde.StatusNotifierItem-4242-1"
    ));
    // A bare id implies the default item path but still names its service.
    assert!(id_matches_service(
        "org.kde.StatusNotifierItem-4242-1",
        "org.kde.StatusNotifierItem-4242-1"
    ));
    assert!(!id_matches_service(":1.42/StatusNotifierItem", ":1.43"));
    assert!(!id_matches_service(":1.42/StatusNotifierItem", ""));
}

#[test]
fn unregistering_a_known_item_removes_only_that_item() {
    let (evt_tx, evt_rx) = quiet_evt_tx();
    let mut known = [
        ":1.10/StatusNotifierItem".to_string(),
        ":1.11/StatusNotifierItem".to_string(),
    ]
    .into_iter()
    .collect::<std::collections::HashSet<_>>();

    handle_unregistered(&evt_tx, &mut known, ":1.10/StatusNotifierItem");

    let events: Vec<_> = evt_rx.try_iter().collect();
    assert_eq!(events.len(), 1);
    match &events[0] {
        super::SystrayEvt::ItemRemoved(service, path) => {
            assert_eq!(service, ":1.10");
            assert_eq!(path, "/StatusNotifierItem");
        }
        other => panic!("expected ItemRemoved, got {other:?}"),
    }
    assert_eq!(
        known,
        [":1.11/StatusNotifierItem".to_string()]
            .into_iter()
            .collect()
    );

    // Unknown ids (never fetched, or already gone) stay silent.
    handle_unregistered(&evt_tx, &mut known, ":1.99/StatusNotifierItem");
    assert!(evt_rx.try_iter().next().is_none());
}

#[test]
fn a_lost_bus_name_removes_its_items_and_prunes_the_watcher_state() {
    let (evt_tx, evt_rx) = quiet_evt_tx();
    let state = Arc::new(Mutex::new(WatcherState::default()));
    state.lock().unwrap().items = vec![
        "org.kde.StatusNotifierItem-4242-1/StatusNotifierItem".to_string(),
        ":1.11/StatusNotifierItem".to_string(),
    ];
    let mut known = state.lock().unwrap().items.clone().into_iter().collect();

    handle_name_lost(
        &WatcherMode::Embedded(Arc::clone(&state)),
        &evt_tx,
        &mut known,
        "org.kde.StatusNotifierItem-4242-1",
    );

    let events: Vec<_> = evt_rx.try_iter().collect();
    assert_eq!(events.len(), 1);
    match &events[0] {
        super::SystrayEvt::ItemRemoved(service, path) => {
            assert_eq!(service, "org.kde.StatusNotifierItem-4242-1");
            assert_eq!(path, "/StatusNotifierItem");
        }
        other => panic!("expected ItemRemoved, got {other:?}"),
    }
    // The surviving item is untouched in both the discovery set and the
    // watcher's advertised list.
    assert_eq!(
        known,
        [":1.11/StatusNotifierItem".to_string()]
            .into_iter()
            .collect()
    );
    assert_eq!(
        state.lock().unwrap().items,
        vec![":1.11/StatusNotifierItem".to_string()]
    );

    // Names with no hosted items are ignored without side effects.
    handle_name_lost(&WatcherMode::Embedded(state), &evt_tx, &mut known, ":1.50");
    assert!(evt_rx.try_iter().next().is_none());
}

// ─────────────────────────────────────────────────────────────────────────────
// End-to-end discovery smoke test
//
// Runs the worker against a throwaway session bus (via `dbus-run-session`)
// and asserts that a registering StatusNotifierItem appears — and a dying one
// disappears — through the signal path in seconds, not by waiting for the
// 10s fallback reconcile. Skips itself when no session bus tooling exists.
// ─────────────────────────────────────────────────────────────────────────────

/// Serve the minimum `StatusNotifierItem` surface the discovery lane reads.
struct FakeItem;

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl FakeItem {
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![(1, 1, vec![0x80, 0x10, 0x20, 0x30])]
    }
}

struct FakeExternalWatcher {
    items: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl FakeExternalWatcher {
    fn register_status_notifier_host(&self, _service: &str) {}

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().unwrap().clone()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}

fn recv_until(
    evt_rx: &std::sync::mpsc::Receiver<super::SystrayEvt>,
    matches: impl FnMut(&super::SystrayEvt) -> bool,
    what: &str,
) -> std::time::Instant {
    let started = std::time::Instant::now();
    let mut matches = matches;
    loop {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{what} was not delivered within 5s — the signal path is broken and only \
             the 10s fallback reconcile would have caught it",
        );
        match evt_rx.recv_timeout(Duration::from_millis(20)) {
            Ok(evt) if matches(&evt) => return started,
            Ok(_) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("worker stopped while waiting for {what}")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
        }
    }
}

/// Child half of the smoke test: runs under the private bus set up by the
/// parent and drives one item through registration and death.
fn sni_smoke_child() {
    let worker = StatusNotifierWorker::spawn(None, None).expect("spawn systray worker");

    let boot = std::time::Instant::now();
    loop {
        assert!(
            boot.elapsed() < Duration::from_secs(10),
            "worker never became ready"
        );
        match worker.evt_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(super::SystrayEvt::Ready) => break,
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("worker stopped before becoming ready")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
    }

    // An SNI app: serve the item object, then hand its path to the watcher.
    let item_conn = zbus::blocking::Connection::session().expect("item connects to the bus");
    item_conn
        .object_server()
        .at("/StatusNotifierItem", FakeItem)
        .expect("serve fake item");
    {
        let watcher = super::uncached_proxy(
            &item_conn,
            super::WATCHER_SERVICE,
            super::WATCHER_PATH,
            super::WATCHER_IFACE,
        )
        .expect("watcher proxy");
        watcher
            .call::<_, _, ()>("RegisterStatusNotifierItem", &("/StatusNotifierItem",))
            .expect("register fake item");
    }

    let registered_at = recv_until(
        &worker.evt_rx,
        |evt| matches!(evt, super::SystrayEvt::ItemUpsert(_)),
        "the registered item's icon",
    );
    let upsert_ms = registered_at.elapsed().as_millis();
    println!("SMOKE upsert after {upsert_ms}ms");

    // A broad signal match replaces the old per-item watcher threads. Verify
    // that the unique D-Bus sender is mapped back to the registered item.
    item_conn
        .emit_signal(
            None::<&str>,
            "/StatusNotifierItem",
            super::ITEM_IFACE,
            "NewIcon",
            &(),
        )
        .expect("emit NewIcon");
    let icon_at = recv_until(
        &worker.evt_rx,
        |evt| matches!(evt, super::SystrayEvt::ItemUpsert(_)),
        "the changed icon",
    );
    println!(
        "SMOKE icon refresh after {}ms",
        icon_at.elapsed().as_millis()
    );

    // The app exits: closing its connection releases the bus name, which the
    // worker must notice via NameOwnerChanged — not via the fallback.
    drop(item_conn);
    let died_at = recv_until(
        &worker.evt_rx,
        |evt| matches!(evt, super::SystrayEvt::ItemRemoved(_, _)),
        "the dead item's removal",
    );
    println!("SMOKE removal after {}ms", died_at.elapsed().as_millis());

    // Closing the command lane must also close and join the owned signal
    // listener. A leaked blocking watcher would hang this join until the
    // parent test's deadline kills the process.
    let StatusNotifierWorker {
        cmd_tx,
        evt_rx,
        thread,
    } = worker;
    drop(cmd_tx);
    drop(evt_rx);
    thread.join().expect("worker shuts down cleanly");
    println!("SMOKE worker stopped");
}

fn external_sni_smoke_child() {
    let items = Arc::new(Mutex::new(Vec::new()));
    let watcher_conn = zbus::blocking::Connection::session().expect("watcher connects to bus");
    watcher_conn
        .object_server()
        .at(
            super::WATCHER_PATH,
            FakeExternalWatcher {
                items: Arc::clone(&items),
            },
        )
        .expect("serve external watcher");
    watcher_conn
        .request_name(super::WATCHER_SERVICE)
        .expect("own watcher name");

    let worker = StatusNotifierWorker::spawn(None, None).expect("spawn systray worker");
    recv_until(
        &worker.evt_rx,
        |evt| matches!(evt, super::SystrayEvt::Ready),
        "external worker readiness",
    );

    let item_conn = zbus::blocking::Connection::session().expect("item connects to bus");
    item_conn
        .object_server()
        .at("/StatusNotifierItem", FakeItem)
        .expect("serve fake item");
    let id = format!(
        "{}/StatusNotifierItem",
        item_conn
            .unique_name()
            .expect("item has a unique name")
            .as_str()
    );
    items.lock().unwrap().push(id.clone());
    watcher_conn
        .emit_signal(
            None::<&str>,
            super::WATCHER_PATH,
            super::WATCHER_IFACE,
            "StatusNotifierItemRegistered",
            &id,
        )
        .expect("emit external registration");
    recv_until(
        &worker.evt_rx,
        |evt| matches!(evt, super::SystrayEvt::ItemUpsert(_)),
        "external registration",
    );

    watcher_conn
        .emit_signal(
            None::<&str>,
            super::WATCHER_PATH,
            super::WATCHER_IFACE,
            "StatusNotifierItemRegistered",
            &id,
        )
        .expect("emit duplicate registration");
    assert!(
        worker
            .evt_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err(),
        "duplicate registration triggered another icon fetch",
    );

    items.lock().unwrap().clear();
    watcher_conn
        .emit_signal(
            None::<&str>,
            super::WATCHER_PATH,
            super::WATCHER_IFACE,
            "StatusNotifierItemUnregistered",
            &id,
        )
        .expect("emit external unregistration");
    recv_until(
        &worker.evt_rx,
        |evt| matches!(evt, super::SystrayEvt::ItemRemoved(_, _)),
        "external unregistration",
    );

    let StatusNotifierWorker {
        cmd_tx,
        evt_rx,
        thread,
    } = worker;
    drop(cmd_tx);
    drop(evt_rx);
    thread.join().expect("external worker shuts down cleanly");
    drop(item_conn);
    drop(watcher_conn);
    println!("EXTERNAL SMOKE passed");
}

#[test]
fn items_appear_and_vanish_via_signals_not_the_fallback() {
    if std::env::var_os("INSTANTWM_SNI_SMOKE_CHILD").is_some() {
        sni_smoke_child();
        return;
    }
    if !Command::new("dbus-run-session")
        .arg("--version")
        .output()
        .is_ok_and(|ok| ok.status.success())
    {
        // No session-bus tooling in this environment; the lane is still
        // covered by the handler unit tests above.
        return;
    }

    // Re-run this very test inside a throwaway session bus. The worker finds
    // no external watcher there, so this exercises the embedded watcher, its
    // registration hook, and NameOwnerChanged-based death detection.
    let mut child = Command::new("dbus-run-session")
        .arg("--")
        .arg(std::env::current_exe().expect("test binary"))
        .args([
            "items_appear_and_vanish_via_signals_not_the_fallback",
            "--nocapture",
        ])
        .env("INSTANTWM_SNI_SMOKE_CHILD", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn dbus-run-session");

    let deadline = std::time::Instant::now() + Duration::from_secs(45);
    let status = loop {
        match child.try_wait().expect("poll smoke child") {
            Some(status) => break status,
            None if std::time::Instant::now() > deadline => {
                let _ = child.kill();
                panic!("smoke child did not finish within 45s");
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    let output = child.wait_with_output().expect("collect smoke output");
    assert!(
        status.success(),
        "smoke child failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    print!("{stdout}");
    assert!(stdout.contains("SMOKE upsert after"), "no upsert line");
    assert!(
        stdout.contains("SMOKE icon refresh after"),
        "no icon refresh line"
    );
    assert!(stdout.contains("SMOKE removal after"), "no removal line");
    assert!(
        stdout.contains("SMOKE worker stopped"),
        "worker leaked a watcher"
    );
}

#[test]
fn external_watcher_signals_are_subscribed_before_ready() {
    if std::env::var_os("INSTANTWM_SNI_EXTERNAL_CHILD").is_some() {
        external_sni_smoke_child();
        return;
    }
    if !Command::new("dbus-run-session")
        .arg("--version")
        .output()
        .is_ok_and(|ok| ok.status.success())
    {
        return;
    }

    let output = Command::new("dbus-run-session")
        .arg("--")
        .arg(std::env::current_exe().expect("test binary"))
        .args([
            "external_watcher_signals_are_subscribed_before_ready",
            "--nocapture",
        ])
        .env("INSTANTWM_SNI_EXTERNAL_CHILD", "1")
        .output()
        .expect("run external watcher smoke test");
    assert!(
        output.status.success(),
        "external watcher smoke test failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    print!("{stdout}");
    assert!(stdout.contains("EXTERNAL SMOKE passed"));
}
