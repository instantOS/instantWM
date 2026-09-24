use super::*;

pub(super) fn run_item_refresh(
    conn: &Connection,
    mode: &WatcherMode,
    evt_tx: &SystrayEventTx,
    watch_tx: Sender<WatcherEvent>,
    watch_rx: Receiver<WatcherEvent>,
) {
    let mut known_ids = HashSet::new();
    // Install D-Bus match rules before taking the initial snapshot. This
    // closes the gap where an external watcher could announce an item after
    // reconciliation but before the old watcher threads subscribed.
    let signal_watcher = match SignalWatcher::spawn(mode, watch_tx.clone()) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            log::warn!("status notifier: failed to start signal watcher: {error}");
            None
        }
    };
    reconcile_items_for_mode(conn, mode, evt_tx, &mut known_ids);
    if !evt_tx.send(SystrayEvt::Ready) {
        return;
    }

    let mut fallback_deadline = Instant::now() + ICON_REFRESH_FALLBACK;
    loop {
        let idle_for = fallback_deadline.saturating_duration_since(Instant::now());
        match watch_rx.recv_timeout(idle_for) {
            Ok(WatcherEvent::NewIcon(sender, path)) => {
                refresh_signalled_icons(conn, evt_tx, &known_ids, &sender, &path);
            }
            Ok(WatcherEvent::Registered(id)) => {
                handle_registered(conn, evt_tx, &mut known_ids, &id);
            }
            Ok(WatcherEvent::Unregistered(id)) => {
                handle_unregistered(evt_tx, &mut known_ids, &id);
            }
            Ok(WatcherEvent::NameLost(name)) => {
                handle_name_lost(mode, evt_tx, &mut known_ids, &name);
            }
            Ok(WatcherEvent::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                reconcile_items_for_mode(conn, mode, evt_tx, &mut known_ids);
                fallback_deadline = Instant::now() + ICON_REFRESH_FALLBACK;
            }
        }
    }

    drop(signal_watcher);
}

/// One owned D-Bus listener replaces the previous collection of detached
/// blocking threads. Its connection is separate from the worker connection so
/// closing it reliably interrupts the iterator without disturbing commands.
pub(super) fn handle_registered(
    conn: &Connection,
    evt_tx: &SystrayEventTx,
    known_ids: &mut HashSet<String>,
    id: &str,
) {
    if !known_ids.insert(id.to_string()) {
        return;
    }
    let Some((service, path)) = parse_sni_id(id) else {
        known_ids.remove(id);
        return;
    };
    let Some((icon_rgba, icon_size)) = fetch_item_icon_on_conn(conn, &service, &path) else {
        // Icon not available yet. Keep the registration known so NewIcon and
        // the fallback reconcile can retry it.
        return;
    };
    evt_tx.send(SystrayEvt::ItemUpsert(StatusNotifierItem {
        service,
        path,
        icon_rgba,
        icon_size,
    }));
}

/// A tray item unregistered: drop it from the tray immediately.
pub(super) fn handle_unregistered(
    evt_tx: &SystrayEventTx,
    known_ids: &mut HashSet<String>,
    id: &str,
) {
    if !known_ids.remove(id) {
        return;
    }
    if let Some((service, path)) = parse_sni_id(id) {
        evt_tx.send(SystrayEvt::ItemRemoved(service, path));
    }
}

/// A bus name lost its owner: every known item it hosted is gone. Also prunes
/// the embedded watcher's advertised item list so its property stays truthful.
pub(super) fn handle_name_lost(
    mode: &WatcherMode,
    evt_tx: &SystrayEventTx,
    known_ids: &mut HashSet<String>,
    name: &str,
) {
    let dead: Vec<String> = known_ids
        .iter()
        .filter(|id| id_matches_service(id, name))
        .cloned()
        .collect();
    for id in &dead {
        known_ids.remove(id);
        log::info!("status notifier: item {id} lost its bus name");
        if let Some((service, path)) = parse_sni_id(id) {
            evt_tx.send(SystrayEvt::ItemRemoved(service, path));
        }
    }
    if let WatcherMode::Embedded(state) = mode {
        state
            .lock()
            .unwrap()
            .items
            .retain(|id| !id_matches_service(id, name));
    }
}

/// Whether a canonical item id (`service/path`) is hosted by the given bus name.
pub(super) fn id_matches_service(id: &str, service: &str) -> bool {
    parse_sni_id(id).is_some_and(|(id_service, _)| id_service == service)
}

/// Refresh every known item matching the unique sender and object path of a
/// broad NewIcon signal. Well-known item names are resolved only when a signal
/// arrives, keeping idle discovery free of D-Bus calls.
pub(super) fn refresh_signalled_icons(
    conn: &Connection,
    evt_tx: &SystrayEventTx,
    known_ids: &HashSet<String>,
    sender: &str,
    signal_path: &str,
) {
    for id in known_ids {
        let Some((service, path)) = parse_sni_id(id) else {
            continue;
        };
        if path != signal_path {
            continue;
        }
        let matches_sender = service == sender
            || (!service.starts_with(':')
                && uncached_proxy(
                    conn,
                    "org.freedesktop.DBus",
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                )
                .and_then(|proxy| proxy.call::<_, _, String>("GetNameOwner", &(service.as_str(),)))
                .is_ok_and(|owner| owner == sender));
        if matches_sender {
            refresh_item_icon(conn, evt_tx, &service, &path);
        }
    }
}

/// Fetch and re-publish one item's icon on demand (e.g. after `NewIcon`).
pub(super) fn refresh_item_icon(
    conn: &Connection,
    evt_tx: &SystrayEventTx,
    service: &str,
    path: &str,
) {
    let Some((icon_rgba, icon_size)) = fetch_item_icon_on_conn(conn, service, path) else {
        return;
    };
    evt_tx.send(SystrayEvt::ItemUpsert(StatusNotifierItem {
        service: service.to_string(),
        path: path.to_string(),
        icon_rgba,
        icon_size,
    }));
}

/// Probe the session bus for an existing StatusNotifierWatcher.
/// If one exists, use it (external mode). Otherwise start our own (embedded mode).
///
/// `events` feeds registration notifications to the discovery lane when the
/// embedded watcher is used; external mode subscribes to the watcher's
/// signals through [`SignalWatcher`].
pub(super) fn reconcile_items_for_mode(
    conn: &Connection,
    mode: &WatcherMode,
    evt_tx: &SystrayEventTx,
    known_ids: &mut HashSet<String>,
) {
    match mode {
        WatcherMode::External => {
            let _ = reconcile_items(conn, evt_tx, known_ids);
        }
        WatcherMode::Embedded(state) => {
            reconcile_items_embedded(conn, state, evt_tx, known_ids);
        }
    }
}

/// Reconcile items from the embedded watcher's shared state.
pub(super) fn reconcile_items_embedded(
    conn: &Connection,
    state: &Arc<Mutex<WatcherState>>,
    evt_tx: &SystrayEventTx,
    known_ids: &mut HashSet<String>,
) {
    let registered = state.lock().unwrap().items.clone();

    // Prune dead services (app exited without unregistering).
    let dbus_proxy = Proxy::new(
        conn,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    );
    let mut alive = HashSet::new();
    for id in &registered {
        if let Some((service, _path)) = parse_sni_id(id) {
            let is_alive = dbus_proxy
                .as_ref()
                .ok()
                .and_then(|p| {
                    p.call::<_, _, bool>("NameHasOwner", &(service.as_str(),))
                        .ok()
                })
                .unwrap_or(false);
            if is_alive {
                alive.insert(id.clone());
            } else {
                log::info!("embedded watcher: pruning dead item {id}");
            }
        }
    }

    // Remove dead items from watcher state.
    if alive.len() != registered.len() {
        let mut st = state.lock().unwrap();
        st.items.retain(|id| alive.contains(id));
    }

    reconcile_ids(conn, evt_tx, known_ids, alive);
}

pub(super) fn reconcile_ids(
    conn: &Connection,
    evt_tx: &SystrayEventTx,
    known_ids: &mut HashSet<String>,
    ids: impl IntoIterator<Item = String>,
) {
    let mut seen = HashSet::new();
    for id in ids {
        seen.insert(id.clone());
        if let Some((service, path)) = parse_sni_id(&id)
            && let Some((icon_rgba, icon_size)) = fetch_item_icon_on_conn(conn, &service, &path)
        {
            evt_tx.send(SystrayEvt::ItemUpsert(StatusNotifierItem {
                service,
                path,
                icon_rgba,
                icon_size,
            }));
        }
    }

    for removed in known_ids.difference(&seen) {
        if let Some((service, path)) = parse_sni_id(removed) {
            evt_tx.send(SystrayEvt::ItemRemoved(service, path));
        }
    }
    *known_ids = seen;
}

pub(super) fn reconcile_items(
    conn: &Connection,
    evt_tx: &SystrayEventTx,
    known_ids: &mut HashSet<String>,
) -> zbus::Result<()> {
    let proxy = uncached_proxy(conn, WATCHER_SERVICE, WATCHER_PATH, WATCHER_IFACE)?;
    let services: Vec<String> = proxy.get_property("RegisteredStatusNotifierItems")?;
    reconcile_ids(conn, evt_tx, known_ids, services);
    Ok(())
}

pub(super) fn parse_sni_id(id: &str) -> Option<(String, String)> {
    if let Some((service, path)) = id.split_once('/') {
        let full_path = format!("/{path}");
        if service.is_empty() || full_path == "/" {
            return None;
        }
        return Some((service.to_string(), full_path));
    }
    if id.starts_with('/') {
        return None;
    }
    Some((id.to_string(), "/StatusNotifierItem".to_string()))
}
