use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use calloop::ping::Ping;
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

/// How often to fall back to a full reconcile scan of StatusNotifierItems.
///
/// Live icon changes are delivered via the item's `NewIcon` signal so the
/// compositor does not need to poll the (potentially large) `IconPixmap`
/// property every second. This fallback only exists for items that do not
/// emit `NewIcon`, so it runs at a slow, low-cost cadence.
const ICON_REFRESH_FALLBACK: Duration = Duration::from_secs(10);

/// Build a short-lived proxy without zbus' lazy property cache.
///
/// The systray worker reads individual properties while reconciling items and
/// opening menus. Enabling the cache makes the first property read fetch every
/// property with `GetAll` and install a `PropertiesChanged` match, only for the
/// proxy to be dropped immediately afterwards. Some Electron StatusNotifier
/// implementations answer `GetAll` much more slowly than a targeted `Get`, so
/// that unnecessary work can also hold up interactive commands on this worker.
mod icon;
mod menu;
use icon::*;
use menu::*;

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

/// Event transport with an optional event-loop wake.
///
/// Worker threads push model updates through this handle. When the running
/// backend registered a ping (see `crate::runtime::make_wake_ping`), every
/// delivered event wakes the loop so tray changes render immediately instead
/// of waiting for the next unrelated wakeup.
#[derive(Clone)]
struct SystrayEventTx {
    tx: Sender<SystrayEvt>,
    wake: Option<Ping>,
}

impl SystrayEventTx {
    fn send(&self, event: SystrayEvt) -> bool {
        let delivered = self.tx.send(event).is_ok();
        if delivered && let Some(ping) = self.wake.as_ref() {
            ping.ping();
        }
        delivered
    }
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
    fn spawn(
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

pub(crate) struct StatusNotifierRuntime {
    worker: Option<StatusNotifierWorker>,
    restart_at: Option<Instant>,
    retry_delay: Duration,
    next_menu_session_id: AtomicU64,
    native_menu_request: Option<NativeMenuRequestSlot>,
    wake: Option<Ping>,
}

impl StatusNotifierRuntime {
    pub(crate) fn start(
        native_menu_request: Option<NativeMenuRequestSlot>,
        wake: Option<Ping>,
    ) -> Self {
        let mut runtime = Self {
            worker: None,
            restart_at: None,
            retry_delay: WORKER_RETRY_MIN,
            next_menu_session_id: AtomicU64::new(1),
            native_menu_request,
            wake,
        };
        match StatusNotifierWorker::spawn(runtime.native_menu_request.clone(), runtime.wake.clone())
        {
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
        match StatusNotifierWorker::spawn(self.native_menu_request.clone(), self.wake.clone()) {
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
        .name("instantwm-systray-refresh".to_string())
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

    drop(refresh_stop_tx);
    if let Some(thread) = refresh_thread {
        let _ = thread.join();
    }
}

fn run_item_refresh(
    conn: &Connection,
    mode: &WatcherMode,
    evt_tx: &SystrayEventTx,
    stop_rx: Receiver<()>,
) {
    let mut known_ids = HashSet::new();
    let mut watching = HashSet::new();
    reconcile_items_for_mode(conn, mode, evt_tx, &mut known_ids);
    if !evt_tx.send(SystrayEvt::Ready) {
        return;
    }

    // Live icon changes are delivered by each item's `NewIcon` signal, so we
    // do not need to re-download IconPixmap on a tight timer. Watcher threads
    // forward (service, path) here whenever an icon actually changes.
    let (changed_tx, changed_rx) = channel();
    spawn_icon_watchers(conn, &known_ids, &mut watching, &changed_tx);

    let mut fallback_deadline = Instant::now() + ICON_REFRESH_FALLBACK;
    loop {
        match changed_rx.recv_timeout(Duration::from_secs(1)) {
            Ok((service, path)) => {
                refresh_item_icon(conn, evt_tx, &service, &path);
            }
            Err(RecvTimeoutError::Timeout) => {
                // Parent ended the worker by dropping the stop sender, or sent
                // an explicit stop signal.
                match stop_rx.try_recv() {
                    Err(TryRecvError::Empty) => {}
                    Ok(()) | Err(TryRecvError::Disconnected) => break,
                }
                if Instant::now() >= fallback_deadline {
                    reconcile_items_for_mode(conn, mode, evt_tx, &mut known_ids);
                    spawn_icon_watchers(conn, &known_ids, &mut watching, &changed_tx);
                    fallback_deadline = Instant::now() + ICON_REFRESH_FALLBACK;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Spawn a blocking `NewIcon` watcher for each registered item that does not
/// yet have one. Each watcher forwards `(service, path)` when the item emits
/// `NewIcon`, so icons update immediately without the compositor polling.
fn spawn_icon_watchers(
    conn: &Connection,
    ids: &HashSet<String>,
    watching: &mut HashSet<String>,
    changed_tx: &Sender<(String, String)>,
) {
    let new_ids: Vec<&String> = ids.iter().filter(|id| !watching.contains(*id)).collect();
    for id in new_ids {
        let Some((service, path)) = parse_sni_id(id) else {
            continue;
        };
        watching.insert(id.clone());
        let conn = conn.clone();
        let tx = changed_tx.clone();
        let watch_service = service.clone();
        let watch_path = path.clone();
        if std::thread::Builder::new()
            .name("instantwm-sni-watch".to_string())
            .spawn(move || watch_item_icon(conn, tx, watch_service, watch_path))
            .is_err()
        {
            log::warn!("status notifier: failed to spawn icon watcher for {service}{path}");
        }
    }
}

/// Block waiting for the item's `NewIcon` signal, re-emitting its current icon.
///
/// Any failure (item gone, DBus error, no signal support) simply ends this
/// watcher; the item is still covered by the slow fallback reconcile, so this
/// is best-effort and never silently drops an icon forever.
fn watch_item_icon(conn: Connection, tx: Sender<(String, String)>, service: String, path: String) {
    let proxy = match uncached_proxy(&conn, &service, &path, ITEM_IFACE) {
        Ok(proxy) => proxy,
        Err(_) => return,
    };
    let mut signals = match proxy.receive_signal("NewIcon") {
        Ok(signals) => signals,
        Err(_) => return,
    };
    while signals.next().is_some() {
        if tx.send((service.clone(), path.clone())).is_err() {
            return;
        }
    }
}

/// Fetch and re-publish one item's icon on demand (e.g. after `NewIcon`).
fn refresh_item_icon(conn: &Connection, evt_tx: &SystrayEventTx, service: &str, path: &str) {
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
fn reconcile_items_embedded(
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

fn reconcile_ids(
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

fn reconcile_items(
    conn: &Connection,
    evt_tx: &SystrayEventTx,
    known_ids: &mut HashSet<String>,
) -> zbus::Result<()> {
    let proxy = uncached_proxy(conn, WATCHER_SERVICE, WATCHER_PATH, WATCHER_IFACE)?;
    let services: Vec<String> = proxy.get_property("RegisteredStatusNotifierItems")?;
    reconcile_ids(conn, evt_tx, known_ids, services);
    Ok(())
}

fn dispatch_cmd(
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

fn record_native_menu_request(
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

fn clear_native_menu_request(slot: Option<&NativeMenuRequestSlot>) {
    if let Some(slot) = slot
        && let Ok(mut request) = slot.lock()
    {
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

#[cfg(test)]
mod tests;
