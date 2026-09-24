use super::*;

/// Shared state backing the embedded watcher D-Bus service.
#[derive(Default)]
pub(super) struct WatcherState {
    /// Canonical item IDs (e.g. ":1.42/StatusNotifierItem").
    pub(super) items: Vec<String>,
    pub(super) has_host: bool,
}

/// D-Bus interface object served at `/StatusNotifierWatcher`.
///
/// The `Arc<Mutex<WatcherState>>` is required for thread safety because:
/// 1. `StatusNotifierWatcherService` implements a `#[zbus::interface]` whose methods are
///    invoked by zbus from its internal thread pool when D-Bus method calls arrive.
/// 2. Multiple D-Bus clients can send concurrent requests (e.g., apps registering items).
/// 3. The systray thread also accesses this state via `reconcile_items_embedded()`.
///    Without the Mutex, this would introduce data races between the zbus thread pool and the
///    systray thread. RefCell is insufficient because it is not thread-safe (`!Send + !Sync`).
pub(super) struct StatusNotifierWatcherService {
    state: Arc<Mutex<WatcherState>>,
    /// Notifies the discovery lane of registrations as they happen, instead
    /// of waiting for the slow fallback reconcile.
    events: Sender<WatcherEvent>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl StatusNotifierWatcherService {
    fn register_status_notifier_item(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        service: &str,
    ) {
        let sender = hdr.sender().map(|s| s.as_str().to_string());
        let canonical = if service.starts_with('/') {
            // App passed an object path; derive service from the D-Bus sender.
            let svc = sender.unwrap_or_default();
            if svc.is_empty() {
                return;
            }
            format!("{svc}{service}")
        } else if service.contains('/') {
            service.to_string()
        } else {
            format!("{service}/StatusNotifierItem")
        };

        let mut st = self.state.lock().unwrap();
        if !st.items.contains(&canonical) {
            log::info!("embedded watcher: registered item {canonical}");
            st.items.push(canonical.clone());
            let _ = self.events.send(WatcherEvent::Registered(canonical));
        }
    }

    fn register_status_notifier_host(&self, _service: &str) {
        let mut st = self.state.lock().unwrap();
        st.has_host = true;
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.state.lock().unwrap().items.clone()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        self.state.lock().unwrap().has_host
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        signal_emitter: &zbus::object_server::SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        signal_emitter: &zbus::object_server::SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(
        signal_emitter: &zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_unregistered(
        signal_emitter: &zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::Result<()>;
}

/// Watcher operating mode — external (nested) or embedded (DRM).
#[derive(Clone)]
pub(super) enum WatcherMode {
    External,
    Embedded(Arc<Mutex<WatcherState>>),
}

/// A discovery-lane event, fed by the signal listener and the embedded
/// watcher service into the refresh loop's channel.
///
/// Everything flows through one channel so `run_item_refresh` can block
/// indefinitely: without a periodic wakeup there is no idle work, and the
/// loop still reacts the moment a tray item is born, dies, or changes icon.
#[derive(Debug)]
pub(super) enum WatcherEvent {
    /// An item emitted `NewIcon` — (unique sender, path).
    NewIcon(String, String),
    /// An item registered with the watcher (canonical `service/path` id).
    Registered(String),
    /// An item unregistered from the watcher (canonical `service/path` id).
    Unregistered(String),
    /// A bus name lost its owner; every item it owns is gone (embedded mode,
    /// where we are the watcher and the SNI spec offers no unregister call).
    NameLost(String),
    /// The parent worker thread is shutting the discovery lane down.
    Stop,
}

pub(super) struct SignalWatcher {
    connection: Option<Connection>,
    thread: Option<thread::JoinHandle<()>>,
}

impl SignalWatcher {
    pub(super) fn spawn(mode: &WatcherMode, tx: Sender<WatcherEvent>) -> zbus::Result<Self> {
        let connection = Connection::session()?;
        let messages = MessageIterator::from(&connection);
        let dbus = uncached_proxy(
            &connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )?;

        // NewIcon is intentionally broad. A single match and listener handles
        // every item, avoiding one permanent OS thread per tray icon.
        add_match(
            &dbus,
            "type='signal',interface='org.kde.StatusNotifierItem',member='NewIcon'",
        )?;
        match mode {
            WatcherMode::External => add_match(
                &dbus,
                "type='signal',sender='org.kde.StatusNotifierWatcher',interface='org.kde.StatusNotifierWatcher'",
            )?,
            WatcherMode::Embedded(_) => add_match(
                &dbus,
                "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',member='NameOwnerChanged'",
            )?,
        }
        drop(dbus);

        let thread = thread::Builder::new()
            .name("instantwm-sni-signals".to_string())
            .spawn(move || watch_signals(messages, tx))?;
        Ok(Self {
            connection: Some(connection),
            thread: Some(thread),
        })
    }
}

impl Drop for SignalWatcher {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            let _ = connection.close();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(super) fn add_match(proxy: &Proxy<'_>, rule: &str) -> zbus::Result<()> {
    proxy.call("AddMatch", &(rule,))
}

pub(super) fn watch_signals(mut messages: MessageIterator, tx: Sender<WatcherEvent>) {
    for message in &mut messages {
        let Ok(message) = message else {
            return;
        };
        let header = message.header();
        let interface = header.interface().map(|name| name.as_str());
        let member = header.member().map(|name| name.as_str());
        let event = match (interface, member) {
            (Some(WATCHER_IFACE), Some("StatusNotifierItemRegistered")) => message
                .body()
                .deserialize::<String>()
                .ok()
                .map(WatcherEvent::Registered),
            (Some(WATCHER_IFACE), Some("StatusNotifierItemUnregistered")) => message
                .body()
                .deserialize::<String>()
                .ok()
                .map(WatcherEvent::Unregistered),
            (Some("org.freedesktop.DBus"), Some("NameOwnerChanged")) => message
                .body()
                .deserialize::<(String, String, String)>()
                .ok()
                .and_then(|(name, old_owner, new_owner)| {
                    (!old_owner.is_empty() && new_owner.is_empty())
                        .then_some(WatcherEvent::NameLost(name))
                }),
            (Some(ITEM_IFACE), Some("NewIcon")) => {
                header.sender().zip(header.path()).map(|(sender, path)| {
                    WatcherEvent::NewIcon(sender.as_str().to_string(), path.as_str().to_string())
                })
            }
            _ => None,
        };
        if event.is_some_and(|event| tx.send(event).is_err()) {
            return;
        }
    }
}

/// A tray item registered: fetch its icon now so it appears immediately
/// instead of waiting for the fallback reconcile.
pub(super) fn detect_watcher_mode(conn: &Connection, events: Sender<WatcherEvent>) -> WatcherMode {
    // Try to read a property from an existing watcher.
    let has_external = uncached_proxy(conn, WATCHER_SERVICE, WATCHER_PATH, WATCHER_IFACE)
        .and_then(|proxy| proxy.get_property::<i32>("ProtocolVersion"))
        .is_ok();

    if has_external {
        log::info!("status notifier: using external StatusNotifierWatcher");
        return WatcherMode::External;
    }

    // No external watcher — start our embedded one.
    log::info!(
        "status notifier: no external watcher found, starting embedded StatusNotifierWatcher"
    );

    let state = Arc::new(Mutex::new(WatcherState::default()));
    let service = StatusNotifierWatcherService {
        state: Arc::clone(&state),
        events,
    };

    // Serve the interface on the existing connection's object server.
    if let Err(e) = conn.object_server().at(WATCHER_PATH, service) {
        log::error!("embedded watcher: failed to serve interface: {e}");
        // Fall back to external mode (will silently fail to show items).
        return WatcherMode::External;
    }

    // Request the well-known bus name so apps can find us.
    match conn.request_name(WATCHER_SERVICE) {
        Ok(_) => {
            log::info!("embedded watcher: acquired bus name {WATCHER_SERVICE}");
        }
        Err(e) => {
            log::warn!("embedded watcher: failed to acquire bus name: {e}");
            // Someone raced us — fall back to external.
            let _ = conn
                .object_server()
                .remove::<StatusNotifierWatcherService, _>(WATCHER_PATH);
            return WatcherMode::External;
        }
    }

    WatcherMode::Embedded(state)
}

/// Reconcile systray items using either external proxy or embedded shared state.
pub(super) fn register_watcher_host(conn: &Connection) {
    if let Ok(proxy) = Proxy::new(conn, WATCHER_SERVICE, WATCHER_PATH, WATCHER_IFACE) {
        let Some(unique_name) = conn.unique_name().map(|n| n.to_string()) else {
            log::warn!("status notifier: cannot register watcher host, missing unique bus name");
            return;
        };
        if let Err(e) = proxy.call::<_, _, ()>("RegisterStatusNotifierHost", &(unique_name)) {
            log::warn!("status notifier: failed to register watcher host: {}", e);
        }
    }
}
