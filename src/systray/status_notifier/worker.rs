use super::*;

#[derive(Debug)]
pub(super) enum SystrayCmd {
    Activate {
        service: String,
        path: String,
        position: Point,
    },
    SecondaryActivate {
        service: String,
        path: String,
        position: Point,
    },
    ContextMenu {
        session_id: u64,
        service: String,
        path: String,
        position: Point,
    },
    MenuAction {
        session_id: u64,
        action: MenuAction,
    },
    CloseMenu {
        session_id: u64,
    },
}

#[derive(Debug)]
pub(super) enum SystrayEvt {
    Ready,
    ItemUpsert(StatusNotifierItem),
    ItemRemoved(String, String),
    MenuChanged {
        session_id: u64,
        view: Option<MenuView>,
    },
}

/// Event transport with an optional event-loop wake.
///
/// Worker threads push model updates through this handle. When the running
/// backend registered a ping (see `crate::runtime::make_wake_ping`), every
/// delivered event wakes the loop so tray changes render immediately instead
/// of waiting for the next unrelated wakeup.
#[derive(Clone)]
pub(super) struct SystrayEventTx {
    pub(super) tx: Sender<SystrayEvt>,
    pub(super) wake: Option<Ping>,
}

impl SystrayEventTx {
    pub(super) fn send(&self, event: SystrayEvt) -> bool {
        let delivered = self.tx.send(event).is_ok();
        if delivered && let Some(ping) = self.wake.as_ref() {
            ping.ping();
        }
        delivered
    }
}

pub(super) struct StatusNotifierWorker {
    pub(super) cmd_tx: Sender<SystrayCmd>,
    pub(super) evt_rx: Receiver<SystrayEvt>,
    pub(super) thread: thread::JoinHandle<()>,
}

impl StatusNotifierWorker {
    pub(super) fn spawn(
        native_menu_request: Option<NativeMenuRequestSlot>,
        wake: Option<Ping>,
    ) -> std::io::Result<Self> {
        let (cmd_tx, cmd_rx) = channel::<SystrayCmd>();
        let (evt_tx, evt_rx) = channel::<SystrayEvt>();
        let evt_tx = SystrayEventTx { tx: evt_tx, wake };
        let thread = thread::Builder::new()
            .name("instantwm-systray".to_string())
            .spawn(move || run_systray_thread(cmd_rx, evt_tx, native_menu_request))?;
        Ok(Self {
            cmd_tx,
            evt_rx,
            thread,
        })
    }
}

pub(super) fn run_systray_thread(
    cmd_rx: Receiver<SystrayCmd>,
    evt_tx: SystrayEventTx,
    native_menu_request: Option<NativeMenuRequestSlot>,
) {
    let conn = match Connection::session() {
        Ok(c) => c,
        Err(e) => {
            log::error!(
                "status notifier: no session bus: {}. Check DBUS_SESSION_BUS_ADDRESS is set",
                e
            );
            return;
        }
    };

    log::info!("status notifier: connected to session bus");

    // One channel feeds the discovery lane: item lifetime signals, icon
    // changes, and shutdown all arrive as `WatcherEvent`s so the refresh
    // thread can block indefinitely instead of polling a stop channel.
    let (watch_tx, watch_rx) = channel::<WatcherEvent>();

    let mode = detect_watcher_mode(&conn, watch_tx.clone());

    match &mode {
        WatcherMode::External => {
            register_watcher_host(&conn);
        }
        WatcherMode::Embedded(state) => {
            // Mark ourselves as a registered host.
            state.lock().unwrap().has_host = true;
        }
    }

    // Icon discovery is deliberately isolated from interactive commands.
    // StatusNotifier items can take a long time to serialize IconPixmap; a
    // slow background refresh must not delay Activate or ContextMenu.
    let refresh_conn = conn.clone();
    let refresh_mode = mode.clone();
    let refresh_evt_tx = evt_tx.clone();
    let refresh_watch_tx = watch_tx.clone();
    let refresh_thread = match thread::Builder::new()
        .name("instantwm-systray-refresh".to_string())
        .spawn(move || {
            run_item_refresh(
                &refresh_conn,
                &refresh_mode,
                &refresh_evt_tx,
                refresh_watch_tx,
                watch_rx,
            );
        }) {
        Ok(thread) => Some(thread),
        Err(error) => {
            log::warn!("status notifier: failed to spawn refresh thread: {error}");
            evt_tx.send(SystrayEvt::Ready);
            None
        }
    };

    let mut menu_session = None;
    let refresh_interval = Duration::from_secs(1);
    let mut next_refresh = Instant::now() + refresh_interval;

    loop {
        let command = if menu_session.is_some() {
            let timeout = next_refresh.saturating_duration_since(Instant::now());
            cmd_rx.recv_timeout(timeout)
        } else {
            // Native menus refresh themselves, and without a hosted DBusMenu
            // there is no periodic work on the interactive lane. Sleep until
            // an actual command arrives rather than adding an idle wakeup.
            cmd_rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match command {
            Ok(cmd) => {
                dispatch_cmd(
                    &conn,
                    cmd,
                    &evt_tx,
                    &mut menu_session,
                    native_menu_request.as_ref(),
                );
                while let Ok(cmd) = cmd_rx.try_recv() {
                    dispatch_cmd(
                        &conn,
                        cmd,
                        &evt_tx,
                        &mut menu_session,
                        native_menu_request.as_ref(),
                    );
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if Instant::now() >= next_refresh {
            refresh_menu_session(&conn, &evt_tx, &mut menu_session);
            next_refresh = Instant::now() + refresh_interval;
        }
    }

    // Watcher threads keep sender clones alive, so channel disconnect cannot
    // signal shutdown; stop the refresh loop explicitly before joining it.
    let _ = watch_tx.send(WatcherEvent::Stop);
    if let Some(thread) = refresh_thread {
        let _ = thread.join();
    }
}

pub(super) fn dispatch_cmd(
    conn: &Connection,
    cmd: SystrayCmd,
    evt_tx: &SystrayEventTx,
    menu_session: &mut Option<DbusMenuSession>,
    native_menu_request: Option<&NativeMenuRequestSlot>,
) {
    match cmd {
        SystrayCmd::Activate {
            service,
            path,
            position,
        } => {
            if let Err(error) = call_item_method(conn, &service, &path, "Activate", position) {
                log::warn!("status notifier: Activate failed for {service}{path}: {error}");
            }
        }
        SystrayCmd::SecondaryActivate {
            service,
            path,
            position,
        } => {
            if let Err(error) =
                call_item_method(conn, &service, &path, "SecondaryActivate", position)
            {
                log::warn!(
                    "status notifier: SecondaryActivate failed for {service}{path}: {error}"
                );
            }
        }
        SystrayCmd::ContextMenu {
            session_id,
            service,
            path,
            position,
        } => {
            if let Some(slot) = native_menu_request
                && let Ok(mut request) = slot.lock()
            {
                *request = None;
            }
            match open_dbus_menu(conn, session_id, &service, &path) {
                Ok(Some(session)) => {
                    let view = session.last_view.clone();
                    *menu_session = Some(session);
                    send_menu_changed(evt_tx, session_id, Some(view));
                }
                Ok(None) => {
                    *menu_session = None;
                    send_menu_changed(evt_tx, session_id, None);
                    // Without a host slot there is no way to claim the item's
                    // own menu toplevel; the item still opens it natively and
                    // positions it itself (the X11 case).
                    record_native_menu_request(
                        conn,
                        native_menu_request,
                        position,
                        &service,
                        &path,
                    );
                    if let Err(error) =
                        call_item_method(conn, &service, &path, "ContextMenu", position)
                    {
                        clear_native_menu_request(native_menu_request);
                        log::warn!(
                            "status notifier: ContextMenu failed for {service}{path}: {error}"
                        );
                    }
                }
                Err(error) => {
                    log::warn!("status notifier: failed to read menu for {service}{path}: {error}");
                    *menu_session = None;
                    send_menu_changed(evt_tx, session_id, None);
                    record_native_menu_request(
                        conn,
                        native_menu_request,
                        position,
                        &service,
                        &path,
                    );
                    if call_item_method(conn, &service, &path, "ContextMenu", position).is_err() {
                        clear_native_menu_request(native_menu_request);
                    }
                }
            }
        }
        SystrayCmd::MenuAction { session_id, action } => {
            if menu_session
                .as_ref()
                .is_some_and(|session| session.id == session_id)
            {
                handle_menu_action(conn, action, evt_tx, menu_session);
            }
        }
        SystrayCmd::CloseMenu { session_id } => {
            if menu_session
                .as_ref()
                .is_some_and(|session| session.id == session_id)
            {
                *menu_session = None;
                send_menu_changed(evt_tx, session_id, None);
            }
        }
    }
}

pub(super) fn record_native_menu_request(
    conn: &Connection,
    slot: Option<&NativeMenuRequestSlot>,
    position: Point,
    service: &str,
    path: &str,
) {
    // Resolving the owner PID is only useful when the compositor will claim
    // the item's menu toplevel by PID; skip the D-Bus round trip otherwise.
    let Some(slot) = slot else {
        return;
    };
    let owner_pid = Proxy::new(
        conn,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .and_then(|proxy| proxy.call("GetConnectionUnixProcessID", &(service,)))
    .ok();
    set_native_menu_request(slot, position, service, path, owner_pid);
}

pub(super) fn set_native_menu_request(
    slot: &NativeMenuRequestSlot,
    position: Point,
    service: &str,
    path: &str,
    owner_pid: Option<u32>,
) {
    if let Ok(mut request) = slot.lock() {
        *request = Some(NativeMenuRequest {
            created: Instant::now(),
            anchor: position,
            service: service.to_string(),
            path: path.to_string(),
            owner_pid,
        });
    }
}

pub(super) fn clear_native_menu_request(slot: Option<&NativeMenuRequestSlot>) {
    if let Some(slot) = slot
        && let Ok(mut request) = slot.lock()
    {
        *request = None;
    }
}

pub(super) fn call_item_method(
    conn: &Connection,
    service: &str,
    path: &str,
    method: &str,
    position: Point,
) -> zbus::Result<()> {
    let proxy = Proxy::new(conn, service, path, ITEM_IFACE)?;
    let _: () = proxy.call(method, &(position.x, position.y))?;
    Ok(())
}

pub(super) fn upsert_item(tray: &mut StatusNotifierTray, item: StatusNotifierItem) -> bool {
    if let Some(existing) = tray
        .items
        .iter_mut()
        .find(|it| it.service == item.service && it.path == item.path)
    {
        let was_changed =
            existing.icon_size != item.icon_size || existing.icon_rgba != item.icon_rgba;
        *existing = item;
        return was_changed;
    }

    tray.items.push(item);
    true
}
