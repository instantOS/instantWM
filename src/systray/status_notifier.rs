use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use zbus::blocking::{Connection, Proxy};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{OwnedValue, Value};

use crate::systray::{
    MenuAction, MenuEntry, MenuToggle, MenuView, StatusNotifierItem, StatusNotifierTray,
};
use crate::types::{MonitorId, MouseButton, Point, Size, TagMask, WindowId};

const WATCHER_SERVICE: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_IFACE: &str = "org.kde.StatusNotifierWatcher";

const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const DBUSMENU_IFACE: &str = "com.canonical.dbusmenu";
const WORKER_RETRY_MIN: Duration = Duration::from_secs(1);
const WORKER_RETRY_MAX: Duration = Duration::from_secs(60);

/// Build a short-lived proxy without zbus' lazy property cache.
///
/// The systray worker reads individual properties while reconciling items and
/// opening menus. Enabling the cache makes the first property read fetch every
/// property with `GetAll` and install a `PropertiesChanged` match, only for the
/// proxy to be dropped immediately afterwards. Some Electron StatusNotifier
/// implementations answer `GetAll` much more slowly than a targeted `Get`, so
/// that unnecessary work can also hold up interactive commands on this worker.
fn uncached_proxy<'a>(
    conn: &Connection,
    destination: &'a str,
    path: &'a str,
    interface: &'a str,
) -> zbus::Result<Proxy<'a>> {
    zbus::blocking::proxy::Builder::new(conn)
        .destination(destination)?
        .path(path)?
        .interface(interface)?
        .cache_properties(CacheProperties::No)
        .build()
}

/// A request expected to produce a native Wayland toplevel because the item
/// does not expose a host-renderable DBusMenu.
#[derive(Clone, Debug)]
pub(crate) struct NativeMenuRequest {
    pub created: Instant,
    pub anchor: Point,
    pub service: String,
    pub path: String,
    /// PID owning the D-Bus name, used to avoid claiming an unrelated
    /// toplevel that happens to map during the request timeout.
    pub owner_pid: Option<u32>,
}

impl NativeMenuRequest {
    pub(crate) fn matches_client_pid(&self, client_pid: Option<u32>) -> bool {
        self.owner_pid
            .is_some_and(|expected| client_pid == Some(expected))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActiveNativeMenu {
    pub win: WindowId,
    pub service: String,
    pub path: String,
    pub monitor_id: MonitorId,
    pub opened_tags: TagMask,
    pub close_requested: bool,
}

/// Cross-thread handoff for a pending native menu request.
pub(crate) type NativeMenuRequestSlot = Arc<Mutex<Option<NativeMenuRequest>>>;

// ─────────────────────────────────────────────────────────────────────────────
// Embedded StatusNotifierWatcher
// ─────────────────────────────────────────────────────────────────────────────

/// Shared state backing the embedded watcher D-Bus service.
#[derive(Default)]
struct WatcherState {
    /// Canonical item IDs (e.g. ":1.42/StatusNotifierItem").
    items: Vec<String>,
    has_host: bool,
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
struct StatusNotifierWatcherService {
    state: Arc<Mutex<WatcherState>>,
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
            st.items.push(canonical);
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
enum WatcherMode {
    External,
    Embedded(Arc<Mutex<WatcherState>>),
}

#[derive(Debug)]
enum SystrayCmd {
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
enum SystrayEvt {
    Ready,
    ItemUpsert(StatusNotifierItem),
    ItemRemoved(String, String),
    MenuChanged {
        session_id: u64,
        view: Option<MenuView>,
    },
}

struct DbusMenuSession {
    id: u64,
    service: String,
    menu_path: String,
    parents: Vec<i32>,
    last_view: MenuView,
}

impl DbusMenuSession {
    fn parent_id(&self) -> i32 {
        self.parents.last().copied().unwrap_or(0)
    }
}

struct StatusNotifierWorker {
    cmd_tx: Sender<SystrayCmd>,
    evt_rx: Receiver<SystrayEvt>,
    thread: thread::JoinHandle<()>,
}

impl StatusNotifierWorker {
    fn spawn(native_menu_request: NativeMenuRequestSlot) -> std::io::Result<Self> {
        let (cmd_tx, cmd_rx) = channel::<SystrayCmd>();
        let (evt_tx, evt_rx) = channel::<SystrayEvt>();
        let thread = thread::Builder::new()
            .name("instantwm-wayland-systray".to_string())
            .spawn(move || run_systray_thread(cmd_rx, evt_tx, native_menu_request))?;
        Ok(Self {
            cmd_tx,
            evt_rx,
            thread,
        })
    }
}

pub(crate) struct StatusNotifierRuntime {
    worker: Option<StatusNotifierWorker>,
    restart_at: Option<Instant>,
    retry_delay: Duration,
    next_menu_session_id: AtomicU64,
    native_menu_request: NativeMenuRequestSlot,
}

impl StatusNotifierRuntime {
    pub(crate) fn start(native_menu_request: NativeMenuRequestSlot) -> Self {
        let mut runtime = Self {
            worker: None,
            restart_at: None,
            retry_delay: WORKER_RETRY_MIN,
            next_menu_session_id: AtomicU64::new(1),
            native_menu_request,
        };
        match StatusNotifierWorker::spawn(Arc::clone(&runtime.native_menu_request)) {
            Ok(worker) => runtime.worker = Some(worker),
            Err(error) => {
                log::warn!("status notifier: failed to spawn thread: {error}");
                runtime.schedule_restart();
            }
        }
        runtime
    }

    pub(crate) fn poll_events(
        &mut self,
        tray: &mut StatusNotifierTray,
        menu: &mut crate::systray::TrayMenuState,
    ) -> bool {
        let mut changed = false;
        let mut worker_stopped = false;
        if let Some(worker) = self.worker.as_ref() {
            loop {
                match worker.evt_rx.try_recv() {
                    Ok(SystrayEvt::Ready) => self.retry_delay = WORKER_RETRY_MIN,
                    Ok(SystrayEvt::ItemUpsert(item)) => {
                        changed |= upsert_item(tray, item);
                    }
                    Ok(SystrayEvt::ItemRemoved(service, path)) => {
                        let before = tray.items.len();
                        tray.items
                            .retain(|it| !(it.service == service && it.path == path));
                        changed |= tray.items.len() != before;
                    }
                    Ok(SystrayEvt::MenuChanged { session_id, view }) => {
                        changed |= menu.apply(session_id, view);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        worker_stopped = true;
                        break;
                    }
                }
            }
            worker_stopped |= worker.thread.is_finished();
        }

        if worker_stopped {
            self.handle_worker_exit();
            changed |= !tray.items.is_empty();
            tray.items.clear();
            changed |= menu.close().is_some();
        }

        if self.worker.is_none()
            && self
                .restart_at
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.restart_worker();
        }
        changed
    }

    fn handle_worker_exit(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        let StatusNotifierWorker {
            cmd_tx,
            evt_rx,
            thread,
        } = worker;
        drop(cmd_tx);
        drop(evt_rx);
        match thread.join() {
            Ok(()) => log::warn!("status notifier: worker stopped; scheduling restart"),
            Err(payload) => log::error!(
                "status notifier: worker panicked: {}; scheduling restart",
                panic_message(payload.as_ref())
            ),
        }
        self.schedule_restart();
    }

    fn schedule_restart(&mut self) {
        self.restart_at = Some(Instant::now() + self.retry_delay);
        self.retry_delay = (self.retry_delay * 2).min(WORKER_RETRY_MAX);
    }

    fn restart_worker(&mut self) {
        match StatusNotifierWorker::spawn(Arc::clone(&self.native_menu_request)) {
            Ok(worker) => {
                log::info!("status notifier: restarting worker");
                self.worker = Some(worker);
                self.restart_at = None;
            }
            Err(error) => {
                log::warn!("status notifier: failed to restart worker: {error}");
                self.schedule_restart();
            }
        }
    }

    pub fn dispatch_click_item(
        &self,
        service: String,
        path: String,
        button: MouseButton,
        position: Point,
    ) -> Option<u64> {
        let mut menu_session_id = None;
        let cmd = match button {
            MouseButton::Left => SystrayCmd::Activate {
                service,
                path,
                position,
            },
            MouseButton::Middle => SystrayCmd::SecondaryActivate {
                service,
                path,
                position,
            },
            MouseButton::Right => {
                let session_id = self.next_menu_session_id.fetch_add(1, Ordering::Relaxed);
                menu_session_id = Some(session_id);
                SystrayCmd::ContextMenu {
                    session_id,
                    service,
                    path,
                    position,
                }
            }
            _ => return None,
        };

        let sent = self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.cmd_tx.send(cmd).is_ok());
        if !sent {
            return None;
        }
        menu_session_id
    }

    pub(crate) fn dispatch_menu_action(&self, session_id: u64, action: MenuAction) {
        if let Some(worker) = self.worker.as_ref() {
            let _ = worker
                .cmd_tx
                .send(SystrayCmd::MenuAction { session_id, action });
        }
    }

    pub(crate) fn close_menu(&self, session_id: u64) {
        if let Some(worker) = self.worker.as_ref() {
            let _ = worker.cmd_tx.send(SystrayCmd::CloseMenu { session_id });
        }
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

fn run_systray_thread(
    cmd_rx: Receiver<SystrayCmd>,
    evt_tx: Sender<SystrayEvt>,
    native_menu_request: NativeMenuRequestSlot,
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

    let mode = detect_watcher_mode(&conn);

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
    let (refresh_stop_tx, refresh_stop_rx) = channel();
    let refresh_conn = conn.clone();
    let refresh_mode = mode.clone();
    let refresh_evt_tx = evt_tx.clone();
    let refresh_thread = match thread::Builder::new()
        .name("instantwm-wayland-systray-refresh".to_string())
        .spawn(move || {
            run_item_refresh(
                &refresh_conn,
                &refresh_mode,
                &refresh_evt_tx,
                refresh_stop_rx,
            );
        }) {
        Ok(thread) => Some(thread),
        Err(error) => {
            log::warn!("status notifier: failed to spawn refresh thread: {error}");
            let _ = evt_tx.send(SystrayEvt::Ready);
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
                dispatch_cmd(&conn, cmd, &evt_tx, &mut menu_session, &native_menu_request);
                while let Ok(cmd) = cmd_rx.try_recv() {
                    dispatch_cmd(&conn, cmd, &evt_tx, &mut menu_session, &native_menu_request);
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

    drop(refresh_stop_tx);
    if let Some(thread) = refresh_thread {
        let _ = thread.join();
    }
}

fn run_item_refresh(
    conn: &Connection,
    mode: &WatcherMode,
    evt_tx: &Sender<SystrayEvt>,
    stop_rx: Receiver<()>,
) {
    let mut known_ids = HashSet::new();
    reconcile_items_for_mode(conn, mode, evt_tx, &mut known_ids);
    if evt_tx.send(SystrayEvt::Ready).is_err() {
        return;
    }

    let refresh_interval = Duration::from_secs(1);
    loop {
        match stop_rx.recv_timeout(refresh_interval) {
            Err(RecvTimeoutError::Timeout) => {
                reconcile_items_for_mode(conn, mode, evt_tx, &mut known_ids);
            }
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Probe the session bus for an existing StatusNotifierWatcher.
/// If one exists, use it (external mode). Otherwise start our own (embedded mode).
fn detect_watcher_mode(conn: &Connection) -> WatcherMode {
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
fn reconcile_items_for_mode(
    conn: &Connection,
    mode: &WatcherMode,
    evt_tx: &Sender<SystrayEvt>,
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
fn reconcile_items_embedded(
    conn: &Connection,
    state: &Arc<Mutex<WatcherState>>,
    evt_tx: &Sender<SystrayEvt>,
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

    // Now reconcile as if we got the items from a proxy.
    let mut seen = HashSet::new();
    for id in &alive {
        seen.insert(id.clone());
        if let Some((service, path)) = parse_sni_id(id)
            && let Some((icon_rgba, icon_size)) = fetch_item_icon_on_conn(conn, &service, &path)
        {
            let _ = evt_tx.send(SystrayEvt::ItemUpsert(StatusNotifierItem {
                service,
                path,
                icon_rgba,
                icon_size,
            }));
        }
    }

    for removed in known_ids.difference(&seen) {
        if let Some((service, path)) = parse_sni_id(removed) {
            let _ = evt_tx.send(SystrayEvt::ItemRemoved(service, path));
        }
    }
    *known_ids = seen;
}

fn reconcile_items(
    conn: &Connection,
    evt_tx: &Sender<SystrayEvt>,
    known_ids: &mut HashSet<String>,
) -> zbus::Result<()> {
    let proxy = uncached_proxy(conn, WATCHER_SERVICE, WATCHER_PATH, WATCHER_IFACE)?;
    let services: Vec<String> = proxy.get_property("RegisteredStatusNotifierItems")?;
    let mut seen = HashSet::new();
    for id in services {
        seen.insert(id.clone());
        if let Some((service, path)) = parse_sni_id(&id)
            && let Some((icon_rgba, icon_size)) = fetch_item_icon_on_conn(conn, &service, &path)
        {
            let _ = evt_tx.send(SystrayEvt::ItemUpsert(StatusNotifierItem {
                service,
                path,
                icon_rgba,
                icon_size,
            }));
        }
    }

    for removed in known_ids.difference(&seen) {
        if let Some((service, path)) = parse_sni_id(removed) {
            let _ = evt_tx.send(SystrayEvt::ItemRemoved(service, path));
        }
    }
    *known_ids = seen;
    Ok(())
}

fn dispatch_cmd(
    conn: &Connection,
    cmd: SystrayCmd,
    evt_tx: &Sender<SystrayEvt>,
    menu_session: &mut Option<DbusMenuSession>,
    native_menu_request: &NativeMenuRequestSlot,
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
            if let Ok(mut request) = native_menu_request.lock() {
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

fn record_native_menu_request(
    conn: &Connection,
    slot: &NativeMenuRequestSlot,
    position: Point,
    service: &str,
    path: &str,
) {
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

fn set_native_menu_request(
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

fn clear_native_menu_request(slot: &NativeMenuRequestSlot) {
    if let Ok(mut request) = slot.lock() {
        *request = None;
    }
}

fn call_item_method(
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

fn open_dbus_menu(
    conn: &Connection,
    session_id: u64,
    service: &str,
    item_path: &str,
) -> zbus::Result<Option<DbusMenuSession>> {
    let item = uncached_proxy(conn, service, item_path, ITEM_IFACE)?;
    let menu_path: String = item
        .get_property("Menu")
        .unwrap_or_else(|_| "/".to_string());
    if menu_path == "/" {
        return Ok(None);
    }

    notify_menu_about_to_show(conn, service, &menu_path, 0);
    let view = fetch_menu_level(conn, service, &menu_path, 0, false)?;
    if view.entries.is_empty() {
        return Ok(None);
    }
    Ok(Some(DbusMenuSession {
        id: session_id,
        service: service.to_string(),
        menu_path,
        parents: Vec::new(),
        last_view: view,
    }))
}

fn handle_menu_action(
    conn: &Connection,
    action: MenuAction,
    evt_tx: &Sender<SystrayEvt>,
    session: &mut Option<DbusMenuSession>,
) {
    let Some(current) = session.as_mut() else {
        return;
    };
    match action {
        MenuAction::Activate(id) => {
            let session_id = current.id;
            if let Err(error) = send_menu_click(conn, &current.service, &current.menu_path, id) {
                log::warn!("status notifier: menu activation failed: {error}");
            }
            *session = None;
            send_menu_changed(evt_tx, session_id, None);
        }
        MenuAction::OpenSubmenu(id) => {
            notify_menu_about_to_show(conn, &current.service, &current.menu_path, id);
            match fetch_menu_level(conn, &current.service, &current.menu_path, id, true) {
                // A submenu level always contains the synthetic Back entry.
                // Do not navigate into a stale submenu that has no real items.
                Ok(view) if view.entries.len() > 1 => {
                    current.parents.push(id);
                    current.last_view = view.clone();
                    send_menu_changed(evt_tx, current.id, Some(view));
                }
                Ok(_) => {}
                Err(error) => log::warn!("status notifier: failed to open submenu: {error}"),
            }
        }
        MenuAction::Back => {
            current.parents.pop();
            let parent_id = current.parent_id();
            notify_menu_about_to_show(conn, &current.service, &current.menu_path, parent_id);
            match fetch_menu_level(
                conn,
                &current.service,
                &current.menu_path,
                parent_id,
                parent_id != 0,
            ) {
                Ok(view) => {
                    current.last_view = view.clone();
                    send_menu_changed(evt_tx, current.id, Some(view));
                }
                Err(error) => log::warn!("status notifier: failed to return to menu: {error}"),
            }
        }
    }
}

fn refresh_menu_session(
    conn: &Connection,
    evt_tx: &Sender<SystrayEvt>,
    session: &mut Option<DbusMenuSession>,
) {
    let Some(current) = session.as_mut() else {
        return;
    };
    let parent_id = current.parent_id();
    match fetch_menu_level(
        conn,
        &current.service,
        &current.menu_path,
        parent_id,
        parent_id != 0,
    ) {
        Ok(view) if view != current.last_view => {
            current.last_view = view.clone();
            send_menu_changed(evt_tx, current.id, Some(view));
        }
        Ok(_) => {}
        Err(error) => {
            let session_id = current.id;
            log::debug!("status notifier: closing unavailable menu: {error}");
            *session = None;
            send_menu_changed(evt_tx, session_id, None);
        }
    }
}

fn send_menu_changed(evt_tx: &Sender<SystrayEvt>, session_id: u64, view: Option<MenuView>) {
    let _ = evt_tx.send(SystrayEvt::MenuChanged { session_id, view });
}

fn notify_menu_about_to_show(conn: &Connection, service: &str, menu_path: &str, id: i32) {
    let Ok(proxy) = Proxy::new(conn, service, menu_path, DBUSMENU_IFACE) else {
        return;
    };
    // Some implementations return the specified boolean while others return
    // an empty body. Keep compatibility without invoking this stateful method
    // twice by deliberately leaving the single reply body uninterpreted.
    let _ = proxy.call_method("AboutToShow", &(id,));
}

fn fetch_menu_level(
    conn: &Connection,
    service: &str,
    menu_path: &str,
    parent_id: i32,
    include_back: bool,
) -> zbus::Result<MenuView> {
    let proxy = Proxy::new(conn, service, menu_path, DBUSMENU_IFACE)?;
    let root = match proxy
        .call::<_, _, (u32, OwnedValue)>("GetLayout", &(parent_id, 1i32, Vec::<String>::new()))
    {
        Ok((_, layout)) => layout,
        Err(_) => {
            let (layout,): (OwnedValue,) =
                proxy.call("GetLayout", &(parent_id, 1i32, Vec::<String>::new()))?;
            layout
        }
    };

    let (_, _, children): (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>) =
        root.try_into().map_err(zbus::Error::Variant)?;
    let mut entries = Vec::with_capacity(children.len() + usize::from(include_back));
    if include_back {
        entries.push(MenuEntry {
            label: "‹ Back".to_string(),
            width: 72,
            enabled: true,
            separator: false,
            toggle: MenuToggle::None,
            action: MenuAction::Back,
        });
    }
    for child in children {
        if let Some(entry) = parse_menu_entry(child)? {
            entries.push(entry);
        }
    }
    Ok(MenuView { entries })
}

fn parse_menu_entry(value: OwnedValue) -> zbus::Result<Option<MenuEntry>> {
    let (id, props, children): (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>) =
        value.try_into().map_err(zbus::Error::Variant)?;
    Ok(menu_entry_from_properties(id, &props, !children.is_empty()))
}

fn menu_entry_from_properties(
    id: i32,
    props: &HashMap<String, OwnedValue>,
    has_layout_children: bool,
) -> Option<MenuEntry> {
    if !menu_prop_bool(props, "visible").unwrap_or(true) {
        return None;
    }

    let separator = menu_prop_string(props, "type").is_some_and(|kind| kind == "separator");
    let raw_label = menu_prop_string(props, "label").unwrap_or_default();
    let label = strip_menu_mnemonics(&raw_label).trim().to_string();
    if !separator && label.is_empty() {
        return None;
    }
    let enabled = !separator && menu_prop_bool(props, "enabled").unwrap_or(true);
    let has_submenu = has_layout_children
        || menu_prop_string(props, "children-display").is_some_and(|value| value == "submenu");
    let toggle_state = menu_prop_i32(props, "toggle-state").unwrap_or(0) == 1;
    let toggle = match menu_prop_string(props, "toggle-type").as_deref() {
        Some("checkmark") => MenuToggle::Check(toggle_state),
        Some("radio") => MenuToggle::Radio(toggle_state),
        _ => MenuToggle::None,
    };
    let indicator_width = if toggle == MenuToggle::None { 0 } else { 16 };
    let submenu_width = if has_submenu { 16 } else { 0 };
    let width = if separator {
        24
    } else {
        (label.chars().count() as i32 * 8 + 20 + indicator_width + submenu_width).max(24)
    };
    Some(MenuEntry {
        label,
        width,
        enabled,
        separator,
        toggle,
        action: if has_submenu {
            MenuAction::OpenSubmenu(id)
        } else {
            MenuAction::Activate(id)
        },
    })
}

fn strip_menu_mnemonics(label: &str) -> String {
    let mut chars = label.chars().peekable();
    let mut stripped = String::with_capacity(label.len());
    while let Some(ch) = chars.next() {
        if ch != '_' {
            stripped.push(ch);
            continue;
        }
        if chars.peek() == Some(&'_') {
            chars.next();
            stripped.push('_');
        }
    }
    stripped
}

fn menu_prop_string(props: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let value = props.get(key)?;
    String::try_from(value.clone())
        .ok()
        .or_else(|| <&str>::try_from(value).ok().map(str::to_string))
}

fn menu_prop_bool(props: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    props.get(key).and_then(|value| bool::try_from(value).ok())
}

fn menu_prop_i32(props: &HashMap<String, OwnedValue>, key: &str) -> Option<i32> {
    props.get(key).and_then(|value| i32::try_from(value).ok())
}

fn send_menu_click(conn: &Connection, service: &str, menu_path: &str, id: i32) -> zbus::Result<()> {
    let proxy = Proxy::new(conn, service, menu_path, DBUSMENU_IFACE)?;
    let _: () = proxy.call("Event", &(id, "clicked", Value::new(""), 0u32))?;
    Ok(())
}

fn register_watcher_host(conn: &Connection) {
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

fn parse_sni_id(id: &str) -> Option<(String, String)> {
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

fn upsert_item(tray: &mut StatusNotifierTray, item: StatusNotifierItem) -> bool {
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

fn fetch_item_icon_on_conn(
    conn: &Connection,
    service: &str,
    path: &str,
) -> Option<(Arc<[u8]>, Size)> {
    let proxy = uncached_proxy(conn, service, path, ITEM_IFACE).ok()?;

    let pixmaps: Vec<(i32, i32, Vec<u8>)> = proxy.get_property("IconPixmap").ok()?;
    if pixmaps.is_empty() {
        return None;
    }

    let (size, bytes) = select_largest_valid_pixmap(pixmaps)?;
    let rgba = dbus_icon_bytes_to_rgba(&bytes, size)?;
    Some((Arc::from(rgba), size))
}

fn select_largest_valid_pixmap(pixmaps: Vec<(i32, i32, Vec<u8>)>) -> Option<(Size, Vec<u8>)> {
    pixmaps
        .into_iter()
        .filter_map(|(width, height, bytes)| {
            let pixels = usize::try_from(width)
                .ok()?
                .checked_mul(usize::try_from(height).ok()?)?;
            let required_bytes = pixels.checked_mul(4)?;
            if bytes.len() < required_bytes {
                return None;
            }
            let area = i64::from(width) * i64::from(height);
            Some((area, width, height, bytes))
        })
        .max_by_key(|(area, _, _, _)| *area)
        .map(|(_, width, height, bytes)| (Size::new(width, height), bytes))
}

fn dbus_icon_bytes_to_rgba(bytes: &[u8], size: Size) -> Option<Vec<u8>> {
    if !size.is_positive() {
        return None;
    }
    let px_count = (size.w as usize).checked_mul(size.h as usize)?;
    let need = px_count.checked_mul(4)?;
    if bytes.len() < need {
        return None;
    }

    let mut out = vec![0u8; need];
    for i in 0..px_count {
        let si = i * 4;
        // StatusNotifierItem::IconPixmap stores ARGB32 pixels in network byte
        // order, so each pixel arrives as A, R, G, B bytes on the wire.
        let a = bytes[si];
        let r = bytes[si + 1];
        let g = bytes[si + 2];
        let b = bytes[si + 3];
        out[si] = r;
        out[si + 1] = g;
        out[si + 2] = b;
        out[si + 3] = a;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
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
}
