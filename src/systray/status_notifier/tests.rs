use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zbus::zvariant::{OwnedValue, Value};

use crate::types::Size;

use super::{
    MenuAction, MenuToggle, NativeMenuRequest, StatusNotifierItem, StatusNotifierRuntime,
    StatusNotifierTray, StatusNotifierWorker, WORKER_RETRY_MIN, clear_native_menu_request,
    dbus_icon_bytes_to_rgba, menu_entry_from_properties, select_largest_valid_pixmap,
    set_native_menu_request, strip_menu_mnemonics,
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

    clear_native_menu_request(&slot);
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
        native_menu_request: Arc::new(Mutex::new(None)),
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
