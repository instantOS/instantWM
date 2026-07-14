use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

use crate::bar::systray::{MenuAction, MenuEntry, MenuToggle, MenuView};
use crate::types::{MouseButton, Point, WaylandSystray, WaylandSystrayItem};

const WATCHER_SERVICE: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_IFACE: &str = "org.kde.StatusNotifierWatcher";

const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const DBUSMENU_IFACE: &str = "com.canonical.dbusmenu";

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
    MenuAction(MenuAction),
    CloseMenu,
}

#[derive(Debug)]
enum SystrayEvt {
    ItemUpsert(WaylandSystrayItem),
    ItemRemoved(String, String),
    MenuChanged(Option<MenuView>),
}

struct DbusMenuSession {
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

    pub(crate) fn poll_events(
        &self,
        wayland_systray: &mut WaylandSystray,
        menu: &mut Option<MenuView>,
    ) -> bool {
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
                Ok(SystrayEvt::MenuChanged(next)) => {
                    if *menu != next {
                        *menu = next;
                        changed = true;
                    }
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

    pub(crate) fn dispatch_menu_action(&self, action: MenuAction) {
        let _ = self.cmd_tx.send(SystrayCmd::MenuAction(action));
    }

    pub(crate) fn close_menu(&self) {
        let _ = self.cmd_tx.send(SystrayCmd::CloseMenu);
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
    let mut menu_session = None;

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            dispatch_cmd(&conn, cmd, &evt_tx, &mut menu_session);
        }

        ticks = ticks.wrapping_add(1);
        if ticks.is_multiple_of(33) {
            reconcile_items_for_mode(&conn, &mode, &evt_tx, &mut known_ids);
            refresh_menu_session(&conn, &evt_tx, &mut menu_session);
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

fn dispatch_cmd(
    conn: &Connection,
    cmd: SystrayCmd,
    evt_tx: &Sender<SystrayEvt>,
    menu_session: &mut Option<DbusMenuSession>,
) {
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
        } => match open_dbus_menu(conn, &service, &path) {
            Ok(Some(session)) => {
                let view = session.last_view.clone();
                *menu_session = Some(session);
                let _ = evt_tx.send(SystrayEvt::MenuChanged(Some(view)));
            }
            Ok(None) => {
                *menu_session = None;
                let _ = evt_tx.send(SystrayEvt::MenuChanged(None));
                if let Err(error) = call_item_method(conn, &service, &path, "ContextMenu", position)
                {
                    log::warn!("wayland systray: ContextMenu failed for {service}{path}: {error}");
                }
            }
            Err(error) => {
                log::warn!("wayland systray: failed to read menu for {service}{path}: {error}");
                *menu_session = None;
                let _ = evt_tx.send(SystrayEvt::MenuChanged(None));
                let _ = call_item_method(conn, &service, &path, "ContextMenu", position);
            }
        },
        SystrayCmd::MenuAction(action) => {
            handle_menu_action(conn, action, evt_tx, menu_session);
        }
        SystrayCmd::CloseMenu => {
            *menu_session = None;
            let _ = evt_tx.send(SystrayEvt::MenuChanged(None));
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

fn open_dbus_menu(
    conn: &Connection,
    service: &str,
    item_path: &str,
) -> zbus::Result<Option<DbusMenuSession>> {
    let item = Proxy::new(conn, service, item_path, ITEM_IFACE)?;
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
            if let Err(error) = send_menu_click(conn, &current.service, &current.menu_path, id) {
                log::warn!("wayland systray: menu activation failed: {error}");
            }
            *session = None;
            let _ = evt_tx.send(SystrayEvt::MenuChanged(None));
        }
        MenuAction::OpenSubmenu(id) => {
            notify_menu_about_to_show(conn, &current.service, &current.menu_path, id);
            match fetch_menu_level(conn, &current.service, &current.menu_path, id, true) {
                // A submenu level always contains the synthetic Back entry.
                // Do not navigate into a stale submenu that has no real items.
                Ok(view) if view.entries.len() > 1 => {
                    current.parents.push(id);
                    current.last_view = view.clone();
                    let _ = evt_tx.send(SystrayEvt::MenuChanged(Some(view)));
                }
                Ok(_) => {}
                Err(error) => log::warn!("wayland systray: failed to open submenu: {error}"),
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
                    let _ = evt_tx.send(SystrayEvt::MenuChanged(Some(view)));
                }
                Err(error) => log::warn!("wayland systray: failed to return to menu: {error}"),
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
            let _ = evt_tx.send(SystrayEvt::MenuChanged(Some(view)));
        }
        Ok(_) => {}
        Err(error) => {
            log::debug!("wayland systray: closing unavailable menu: {error}");
            *session = None;
            let _ = evt_tx.send(SystrayEvt::MenuChanged(None));
        }
    }
}

fn notify_menu_about_to_show(conn: &Connection, service: &str, menu_path: &str, id: i32) {
    let Ok(proxy) = Proxy::new(conn, service, menu_path, DBUSMENU_IFACE) else {
        return;
    };
    if proxy.call::<_, _, bool>("AboutToShow", &(id,)).is_err() {
        let _ = proxy.call::<_, _, ()>("AboutToShow", &(id,));
    }
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
    use std::collections::HashMap;

    use zbus::zvariant::{OwnedValue, Value};

    use super::{
        MenuAction, MenuToggle, dbus_icon_bytes_to_rgba, menu_entry_from_properties,
        select_largest_valid_pixmap, strip_menu_mnemonics,
    };

    fn string_value(value: &str) -> OwnedValue {
        OwnedValue::try_from(Value::from(value)).expect("string is representable as an owned value")
    }

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
}
