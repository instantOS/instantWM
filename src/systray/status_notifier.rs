use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use calloop::ping::Ping;
use zbus::blocking::{Connection, MessageIterator, Proxy};
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
/// Item lifetime and icon changes are delivered as D-Bus signals (see
/// [`WatcherEvent`]), so this scan is only a safety net: it retries items
/// whose icon was not available at registration and covers items that do not
/// emit `NewIcon`. It runs at a slow, low-cost cadence.
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

/// A discovery-lane event, fed by the signal listener and the embedded
/// watcher service into the refresh loop's channel.
///
/// Everything flows through one channel so `run_item_refresh` can block
/// indefinitely: without a periodic wakeup there is no idle work, and the
/// loop still reacts the moment a tray item is born, dies, or changes icon.
#[derive(Debug)]
enum WatcherEvent {
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

fn run_item_refresh(
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
struct SignalWatcher {
    connection: Option<Connection>,
    thread: Option<thread::JoinHandle<()>>,
}

impl SignalWatcher {
    fn spawn(mode: &WatcherMode, tx: Sender<WatcherEvent>) -> zbus::Result<Self> {
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

fn add_match(proxy: &Proxy<'_>, rule: &str) -> zbus::Result<()> {
    proxy.call("AddMatch", &(rule,))
}

fn watch_signals(mut messages: MessageIterator, tx: Sender<WatcherEvent>) {
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
fn handle_registered(
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
        // Icon not available yet. Keep the registration known so duplicate
        // signals stay cheap; the fallback reconcile retries every known id.
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
fn handle_unregistered(evt_tx: &SystrayEventTx, known_ids: &mut HashSet<String>, id: &str) {
    if !known_ids.remove(id) {
        return;
    }
    if let Some((service, path)) = parse_sni_id(id) {
        evt_tx.send(SystrayEvt::ItemRemoved(service, path));
    }
}

/// A bus name lost its owner: every known item it hosted is gone. Also prunes
/// the embedded watcher's advertised item list so its property stays truthful.
fn handle_name_lost(
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
fn id_matches_service(id: &str, service: &str) -> bool {
    parse_sni_id(id).is_some_and(|(id_service, _)| id_service == service)
}

/// Refresh every known item matching the unique sender and object path of a
/// broad NewIcon signal. Well-known item names are resolved only when a signal
/// arrives, keeping idle discovery free of D-Bus calls.
fn refresh_signalled_icons(
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
///
/// `events` feeds registration notifications to the discovery lane when the
/// embedded watcher is used; external mode subscribes to the watcher's
/// signals through [`SignalWatcher`].
fn detect_watcher_mode(conn: &Connection, events: Sender<WatcherEvent>) -> WatcherMode {
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
