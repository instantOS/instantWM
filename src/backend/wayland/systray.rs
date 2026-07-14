use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread;

use zbus::blocking::{Connection, Proxy};

use crate::types::{MouseButton, Point, WaylandSystray, WaylandSystrayItem};

const WATCHER_SERVICE: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_IFACE: &str = "org.kde.StatusNotifierWatcher";

const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";

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
        service: String,
        path: String,
        position: Point,
    },
}

#[derive(Debug)]
enum SystrayEvt {
    ItemUpsert(WaylandSystrayItem),
    ItemRemoved(String, String),
}

pub struct WaylandSystrayRuntime {
    cmd_tx: Sender<SystrayCmd>,
    evt_rx: Receiver<SystrayEvt>,
    /// Track if the systray thread is still running
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl WaylandSystrayRuntime {
    pub fn start() -> Option<Self> {
        let (cmd_tx, cmd_rx) = channel::<SystrayCmd>();
        let (evt_tx, evt_rx) = channel::<SystrayEvt>();

        let builder = thread::Builder::new().name("instantwm-wayland-systray".to_string());
        let spawn = builder.spawn(move || {
            run_systray_thread(cmd_rx, evt_tx);
        });

        let thread_handle = match spawn {
            Ok(handle) => Some(handle),
            Err(e) => {
                log::warn!("wayland systray: failed to spawn thread: {e}");
                return None;
            }
        };

        Some(Self {
            cmd_tx,
            evt_rx,
            thread_handle,
        })
    }

    /// Check if the systray thread is still running
    pub fn is_alive(&self) -> bool {
        self.thread_handle
            .as_ref()
            .map(|h| !h.is_finished())
            .unwrap_or(false)
    }

    pub fn poll_events(&self, wayland_systray: &mut WaylandSystray) -> bool {
        let mut changed = false;
        loop {
            match self.evt_rx.try_recv() {
                Ok(SystrayEvt::ItemUpsert(item)) => {
                    changed |= upsert_item(wayland_systray, item);
                }
                Ok(SystrayEvt::ItemRemoved(service, path)) => {
                    let before = wayland_systray.items.len();
                    wayland_systray
                        .items
                        .retain(|it| !(it.service == service && it.path == path));
                    changed |= wayland_systray.items.len() != before;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        changed
    }

    pub fn dispatch_click_item(
        &self,
        service: String,
        path: String,
        button: MouseButton,
        position: Point,
    ) {
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
            MouseButton::Right => SystrayCmd::ContextMenu {
                service,
                path,
                position,
            },
            _ => return,
        };

        let _ = self.cmd_tx.send(cmd);
    }
}

fn run_systray_thread(cmd_rx: Receiver<SystrayCmd>, evt_tx: Sender<SystrayEvt>) {
    let conn = match Connection::session() {
        Ok(c) => c,
        Err(e) => {
            log::error!(
                "wayland systray: no session bus: {}. Check DBUS_SESSION_BUS_ADDRESS is set",
                e
            );
            return;
        }
    };

    log::info!("wayland systray: connected to session bus");

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

    let mut known_ids: HashSet<String> = HashSet::new();
    reconcile_items_for_mode(&conn, &mode, &evt_tx, &mut known_ids);
    let mut ticks = 0u32;

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            dispatch_cmd(&conn, cmd);
        }

        ticks = ticks.wrapping_add(1);
        if ticks.is_multiple_of(33) {
            reconcile_items_for_mode(&conn, &mode, &evt_tx, &mut known_ids);
        }

        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}

/// Probe the session bus for an existing StatusNotifierWatcher.
/// If one exists, use it (external mode). Otherwise start our own (embedded mode).
fn detect_watcher_mode(conn: &Connection) -> WatcherMode {
    // Try to read a property from an existing watcher.
    let has_external = Proxy::new(conn, WATCHER_SERVICE, WATCHER_PATH, WATCHER_IFACE)
        .and_then(|proxy| proxy.get_property::<i32>("ProtocolVersion"))
        .is_ok();

    if has_external {
        log::info!("wayland systray: using external StatusNotifierWatcher");
        return WatcherMode::External;
    }

    // No external watcher — start our embedded one.
    log::info!(
        "wayland systray: no external watcher found, starting embedded StatusNotifierWatcher"
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
            && let Some((icon_rgba, icon_w, icon_h)) =
                fetch_item_icon_on_conn(conn, &service, &path)
        {
            let _ = evt_tx.send(SystrayEvt::ItemUpsert(WaylandSystrayItem {
                service,
                path,
                icon_rgba,
                icon_w,
                icon_h,
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
    let proxy = Proxy::new(conn, WATCHER_SERVICE, WATCHER_PATH, WATCHER_IFACE)?;
    let services: Vec<String> = proxy.get_property("RegisteredStatusNotifierItems")?;
    let mut seen = HashSet::new();
    for id in services {
        seen.insert(id.clone());
        if let Some((service, path)) = parse_sni_id(&id)
            && let Some((icon_rgba, icon_w, icon_h)) =
                fetch_item_icon_on_conn(conn, &service, &path)
        {
            let _ = evt_tx.send(SystrayEvt::ItemUpsert(WaylandSystrayItem {
                service,
                path,
                icon_rgba,
                icon_w,
                icon_h,
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

fn dispatch_cmd(conn: &Connection, cmd: SystrayCmd) {
    match cmd {
        SystrayCmd::Activate {
            service,
            path,
            position,
        } => {
            if let Err(error) = call_item_method(conn, &service, &path, "Activate", position) {
                log::warn!("wayland systray: Activate failed for {service}{path}: {error}");
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
                    "wayland systray: SecondaryActivate failed for {service}{path}: {error}"
                );
            }
        }
        SystrayCmd::ContextMenu {
            service,
            path,
            position,
        } => {
            if let Err(error) = call_item_method(conn, &service, &path, "ContextMenu", position) {
                log::warn!("wayland systray: ContextMenu failed for {service}{path}: {error}");
            }
        }
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
            log::warn!("wayland systray: cannot register watcher host, missing unique bus name");
            return;
        };
        if let Err(e) = proxy.call::<_, _, ()>("RegisterStatusNotifierHost", &(unique_name)) {
            log::warn!("wayland systray: failed to register watcher host: {}", e);
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

fn upsert_item(wayland_systray: &mut WaylandSystray, item: WaylandSystrayItem) -> bool {
    if let Some(existing) = wayland_systray
        .items
        .iter_mut()
        .find(|it| it.service == item.service && it.path == item.path)
    {
        let was_changed = existing.icon_w != item.icon_w
            || existing.icon_h != item.icon_h
            || existing.icon_rgba != item.icon_rgba;
        *existing = item;
        return was_changed;
    }

    wayland_systray.items.push(item);
    true
}

fn fetch_item_icon_on_conn(
    conn: &Connection,
    service: &str,
    path: &str,
) -> Option<(Arc<[u8]>, i32, i32)> {
    let proxy = Proxy::new(conn, service, path, ITEM_IFACE).ok()?;

    let pixmaps: Vec<(i32, i32, Vec<u8>)> = proxy.get_property("IconPixmap").ok()?;
    if pixmaps.is_empty() {
        return None;
    }

    let (w, h, bytes) = select_largest_valid_pixmap(pixmaps)?;
    let rgba = dbus_icon_bytes_to_rgba(&bytes, w, h)?;
    Some((Arc::from(rgba), w, h))
}

fn select_largest_valid_pixmap(pixmaps: Vec<(i32, i32, Vec<u8>)>) -> Option<(i32, i32, Vec<u8>)> {
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
        .map(|(_, width, height, bytes)| (width, height, bytes))
}

fn dbus_icon_bytes_to_rgba(bytes: &[u8], w: i32, h: i32) -> Option<Vec<u8>> {
    let px_count = (w as usize).checked_mul(h as usize)?;
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
    use super::{dbus_icon_bytes_to_rgba, select_largest_valid_pixmap};

    #[test]
    fn dbus_icon_bytes_are_decoded_from_argb_to_rgba() {
        let bytes = [
            0xff, 0x00, 0x82, 0xc9, // opaque Nextcloud blue
            0x40, 0x11, 0x22, 0x33, // translucent pixel
        ];

        let rgba = dbus_icon_bytes_to_rgba(&bytes, 2, 1).expect("valid icon bytes");

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

        assert_eq!((selected.0, selected.1), (32, 32));
    }
}
