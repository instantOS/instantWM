use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::thread;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

use crate::bar::paint::BarPainter;
use crate::contexts::CoreCtx;
use crate::types::{
    Monitor, MouseButton, WaylandSystray, WaylandSystrayItem, WaylandSystrayMenu,
    WaylandSystrayMenuItem,
};

const WATCHER_SERVICE: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_IFACE: &str = "org.kde.StatusNotifierWatcher";

const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const DBUSMENU_IFACE: &str = "com.canonical.dbusmenu";

#[derive(Debug)]
enum SystrayCmd {
    Activate {
        service: String,
        path: String,
        x: i32,
        y: i32,
    },
    SecondaryActivate {
        service: String,
        path: String,
        x: i32,
        y: i32,
    },
    ContextMenu {
        service: String,
        path: String,
        x: i32,
        y: i32,
    },
    MenuClick {
        service: String,
        path: String,
        id: i32,
    },
}

#[derive(Debug)]
enum SystrayEvt {
    ItemUpsert(WaylandSystrayItem),
    ItemRemoved(String, String),
    MenuOpen(crate::types::WaylandSystrayMenu),
    MenuClose,
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

    pub fn poll_events(
        &self,
        core: &mut CoreCtx,
        wayland_systray: &mut WaylandSystray,
        wayland_systray_menu: &mut Option<WaylandSystrayMenu>,
    ) -> bool {
        let mut changed = false;
        loop {
            match self.evt_rx.try_recv() {
                Ok(SystrayEvt::ItemUpsert(item)) => {
                    changed |= upsert_item(core, wayland_systray, item);
                }
                Ok(SystrayEvt::ItemRemoved(service, path)) => {
                    let before = wayland_systray.items.len();
                    wayland_systray
                        .items
                        .retain(|it| !(it.service == service && it.path == path));
                    changed |= wayland_systray.items.len() != before;
                }
                Ok(SystrayEvt::MenuOpen(menu)) => {
                    *wayland_systray_menu = Some(menu);
                    changed = true;
                }
                Ok(SystrayEvt::MenuClose) => {
                    if wayland_systray_menu.take().is_some() {
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
        root_x: i32,
        root_y: i32,
    ) {
        let cmd = match button {
            MouseButton::Left => SystrayCmd::Activate {
                service,
                path,
                x: root_x,
                y: root_y,
            },
            MouseButton::Middle => SystrayCmd::SecondaryActivate {
                service,
                path,
                x: root_x,
                y: root_y,
            },
            MouseButton::Right => SystrayCmd::ContextMenu {
                service,
                path,
                x: root_x,
                y: root_y,
            },
            _ => return,
        };

        let _ = self.cmd_tx.send(cmd);
    }

    pub fn dispatch_menu_click_item(&self, service: String, path: String, id: i32) {
        let _ = self
            .cmd_tx
            .send(SystrayCmd::MenuClick { service, path, id });
    }
}

pub fn get_wayland_systray_width(core: &CoreCtx) -> i32 {
    if !core.g.cfg.show_systray {
        return 0;
    }
    core.g.systray_width
}

pub fn get_wayland_systray_width_with_state(
    core: &CoreCtx,
    wayland_systray: &WaylandSystray,
) -> i32 {
    if !core.g.cfg.show_systray {
        return 0;
    }
    let items = &wayland_systray.items;
    if items.is_empty() {
        return 0;
    }
    let icon_h = core.g.cfg.bar_height.max(1);
    let spacing = core.g.cfg.systrayspacing.max(0);
    let mut width = spacing;
    for item in items {
        let iw = scale_icon_width(item.icon_w, item.icon_h, icon_h);
        width += iw + spacing;
    }
    width
}

pub fn hit_test_wayland_systray_menu_item(
    core: &CoreCtx,
    wayland_systray: &WaylandSystray,
    wayland_systray_menu: Option<&WaylandSystrayMenu>,
    mon: &Monitor,
    local_x: i32,
) -> Option<usize> {
    let layout = systray_layout(core, wayland_systray, wayland_systray_menu, mon);
    for slot in &layout.menu_slots {
        if local_x >= slot.start && local_x < slot.end {
            return Some(slot.idx);
        }
    }
    None
}

pub fn draw_wayland_systray(
    core: &mut CoreCtx,
    wayland_systray: &WaylandSystray,
    wayland_systray_menu: Option<&WaylandSystrayMenu>,
    mon: &crate::types::Monitor,
    painter: &mut crate::bar::wayland::WaylandBarPainter,
) {
    let layout = systray_layout(core, wayland_systray, wayland_systray_menu, mon);

    // Populate the hit cache with systray slots
    let mon_id = mon.id();
    if let Some(hit) = core.bar.monitor_hit_cache_mut(mon_id) {
        hit.systray_slots = layout
            .tray_slots
            .iter()
            .map(|s| crate::bar::SystrayHitSlot {
                idx: s.idx,
                start: s.start,
                end: s.end,
            })
            .collect();
        hit.systray_menu_slots = layout
            .menu_slots
            .iter()
            .map(|s| crate::bar::SystrayHitSlot {
                idx: s.idx,
                start: s.start,
                end: s.end,
            })
            .collect();
    }

    let bg = core.g.status_scheme().bg;
    let bg_scheme = crate::bar::paint::BarScheme {
        fg: bg,
        bg,
        detail: bg,
    };
    painter.set_scheme(bg_scheme);
    if layout.tray_total_w > 0 {
        painter.rect(
            layout.tray_start_x,
            0,
            layout.tray_total_w,
            core.g.cfg.bar_height,
            true,
            true,
        );
    }
    if layout.menu_total_w > 0 {
        painter.rect(
            layout.menu_start_x,
            0,
            layout.menu_total_w,
            core.g.cfg.bar_height,
            true,
            true,
        );
    }

    let icon_h = core.g.cfg.bar_height.max(1);
    for slot in &layout.tray_slots {
        let Some(item) = wayland_systray.items.get(slot.idx) else {
            continue;
        };
        painter.blit_rgba_bgra(
            slot.start,
            0,
            slot.end - slot.start,
            icon_h,
            item.icon_w,
            item.icon_h,
            &item.icon_rgba,
        );
    }

    if let Some(menu) = wayland_systray_menu {
        draw_menu_overlay(core, painter, menu, &layout);
    }
}

fn scale_icon_width(src_w: i32, src_h: i32, dst_h: i32) -> i32 {
    if src_w <= 0 || src_h <= 0 || dst_h <= 0 {
        return 0;
    }
    if src_h == dst_h {
        src_w.max(1)
    } else {
        ((src_w as f32) * (dst_h as f32 / src_h as f32)).round() as i32
    }
    .max(1)
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

    register_watcher_host(&conn);
    let mut known_ids: HashSet<String> = HashSet::new();
    let _ = reconcile_items(&conn, &evt_tx, &mut known_ids);
    let mut ticks = 0u32;

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            dispatch_cmd(&conn, cmd, &evt_tx);
        }

        ticks = ticks.wrapping_add(1);
        if ticks % 33 == 0 {
            let _ = reconcile_items(&conn, &evt_tx, &mut known_ids);
        }

        std::thread::sleep(std::time::Duration::from_millis(30));
    }
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
        if let Some((service, path)) = parse_sni_id(&id) {
            if let Some((icon_rgba, icon_w, icon_h)) =
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
    }

    for removed in known_ids.difference(&seen) {
        if let Some((service, path)) = parse_sni_id(removed) {
            let _ = evt_tx.send(SystrayEvt::ItemRemoved(service, path));
        }
    }
    *known_ids = seen;
    Ok(())
}

fn dispatch_cmd(conn: &Connection, cmd: SystrayCmd, evt_tx: &Sender<SystrayEvt>) {
    match cmd {
        SystrayCmd::Activate {
            service,
            path,
            x,
            y,
        } => {
            let _ = call_item_method(conn, &service, &path, "Activate", &(x, y));
        }
        SystrayCmd::SecondaryActivate {
            service,
            path,
            x,
            y,
        } => {
            let _ = call_item_method(conn, &service, &path, "SecondaryActivate", &(x, y));
        }
        SystrayCmd::ContextMenu {
            service,
            path,
            x,
            y,
        } => {
            let _ = open_menu_from_item(conn, &service, &path, x, y, evt_tx);
            let _ = call_item_method(conn, &service, &path, "ContextMenu", &(x, y));
        }
        SystrayCmd::MenuClick { service, path, id } => {
            let _ = call_menu_event(conn, &service, &path, id);
            let _ = evt_tx.send(SystrayEvt::MenuClose);
        }
    }
}

fn call_item_method(
    conn: &Connection,
    service: &str,
    path: &str,
    method: &str,
    body: &(i32, i32),
) -> zbus::Result<()> {
    let proxy = Proxy::new(conn, service, path, ITEM_IFACE)?;
    let _: () = proxy.call(method, body)?;
    Ok(())
}

fn register_watcher_host(conn: &Connection) {
    if let Ok(proxy) = Proxy::new(conn, WATCHER_SERVICE, WATCHER_PATH, WATCHER_IFACE) {
        let _: zbus::Result<()> = proxy.call("RegisterStatusNotifierHost", &("instantwm-wayland"));
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

fn systray_layout(
    core: &CoreCtx,
    wayland_systray: &WaylandSystray,
    wayland_systray_menu: Option<&WaylandSystrayMenu>,
    mon: &Monitor,
) -> SystrayLayout {
    let icon_h = core.g.cfg.bar_height.max(1);
    let spacing = core.g.cfg.systrayspacing.max(0);
    let mut tray_total_w = 0;
    if !wayland_systray.items.is_empty() {
        tray_total_w = spacing;
        for item in &wayland_systray.items {
            tray_total_w += scale_icon_width(item.icon_w, item.icon_h, icon_h) + spacing;
        }
    }
    let tray_start_x = mon.work_rect.w - tray_total_w;

    let mut tray_slots = Vec::new();
    let mut x = tray_start_x + spacing;
    for (idx, item) in wayland_systray.items.iter().enumerate() {
        let w = scale_icon_width(item.icon_w, item.icon_h, icon_h);
        if w > 0 && item.icon_w > 0 && item.icon_h > 0 {
            tray_slots.push(HitSlot {
                idx,
                start: x,
                end: x + w,
            });
        }
        x += w + spacing;
    }

    let mut menu_total_w = 0;
    let mut menu_slots = Vec::new();
    let mut menu_start_x = 0;
    if let Some(menu) = wayland_systray_menu {
        for item in &menu.items {
            menu_total_w += item.width.max(24);
        }
        menu_start_x = (tray_start_x - menu_total_w).max(0);
        let mut mx = menu_start_x;
        for (idx, item) in menu.items.iter().enumerate() {
            let w = item.width.max(24);
            menu_slots.push(HitSlot {
                idx,
                start: mx,
                end: mx + w,
            });
            mx += w;
        }
    }

    SystrayLayout {
        tray_total_w,
        tray_start_x,
        tray_slots,
        menu_total_w,
        menu_start_x,
        menu_slots,
    }
}

fn draw_menu_overlay(
    core: &CoreCtx,
    painter: &mut crate::bar::wayland::WaylandBarPainter,
    menu: &WaylandSystrayMenu,
    layout: &SystrayLayout,
) {
    if layout.menu_slots.is_empty() {
        return;
    }
    let mut scheme = core.g.status_scheme();
    painter.set_scheme(scheme.clone());
    let item_h = core.g.cfg.bar_height.max(1);
    for (row, item) in menu.items.iter().enumerate() {
        let Some(slot) = layout.menu_slots.get(row) else {
            continue;
        };
        let x = slot.start;
        let w = slot.end - slot.start;
        let y = 0;
        if item.separator {
            painter.rect(x + 3, y + item_h / 2, w - 6, 1, true, false);
            continue;
        }
        if !item.enabled {
            scheme.fg[3] = 0.6;
            painter.set_scheme(scheme.clone());
        }
        painter.text(x, y, w, item_h, 8, &item.label, false, 0);
        if !item.enabled {
            scheme.fg[3] = 1.0;
            painter.set_scheme(scheme.clone());
        }
    }
}

struct HitSlot {
    idx: usize,
    start: i32,
    end: i32,
}

struct SystrayLayout {
    tray_total_w: i32,
    tray_start_x: i32,
    tray_slots: Vec<HitSlot>,
    menu_total_w: i32,
    menu_start_x: i32,
    menu_slots: Vec<HitSlot>,
}

fn open_menu_from_item(
    conn: &Connection,
    service: &str,
    path: &str,
    _x: i32,
    _y: i32,
    evt_tx: &Sender<SystrayEvt>,
) -> zbus::Result<()> {
    let menu_path: String = Proxy::new(conn, service, path, ITEM_IFACE)?
        .get_property("Menu")
        .unwrap_or_else(|_| "/".to_string());
    if menu_path == "/" {
        return Ok(());
    }

    let menu_items = fetch_menu_items(conn, service, &menu_path)?;
    let menu = WaylandSystrayMenu {
        service: service.to_string(),
        path: menu_path,
        item_h: 0,
        items: menu_items,
    };
    let _ = evt_tx.send(SystrayEvt::MenuOpen(menu));
    Ok(())
}

fn fetch_menu_items(
    conn: &Connection,
    service: &str,
    menu_path: &str,
) -> zbus::Result<Vec<WaylandSystrayMenuItem>> {
    let proxy = Proxy::new(conn, service, menu_path, DBUSMENU_IFACE)?;
    let root = match proxy
        .call::<_, _, (u32, OwnedValue)>("GetLayout", &(0i32, -1i32, Vec::<String>::new()))
    {
        Ok((_, layout)) => layout,
        Err(_) => {
            let (layout,): (OwnedValue,) =
                proxy.call("GetLayout", &(0i32, -1i32, Vec::<String>::new()))?;
            layout
        }
    };

    let mut out = Vec::new();
    parse_menu_layout_node(root, &mut out)?;
    out.retain(|it| it.id > 0);
    Ok(out)
}

fn parse_menu_layout_node(
    value: OwnedValue,
    out: &mut Vec<WaylandSystrayMenuItem>,
) -> zbus::Result<()> {
    let (id, props, children): (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>) =
        value.try_into().map_err(zbus::Error::Variant)?;

    let visible = dbusmenu_prop_bool(&props, "visible").unwrap_or(true);
    let enabled = dbusmenu_prop_bool(&props, "enabled").unwrap_or(true);
    let raw_label = dbusmenu_prop_string(&props, "label").unwrap_or_default();
    let separator = dbusmenu_prop_string(&props, "type")
        .map(|t| t == "separator")
        .unwrap_or(false);

    if visible && (separator || !raw_label.trim().is_empty()) {
        let label = raw_label.replace('_', "").trim().to_string();
        let display = if label.is_empty() && separator {
            "-".to_string()
        } else {
            label
        };
        out.push(WaylandSystrayMenuItem {
            id,
            label: display.clone(),
            width: ((display.chars().count() as i32) * 8 + 20).max(24),
            enabled,
            separator,
        });
    }

    for child in children {
        let _ = parse_menu_layout_node(child, out);
    }

    Ok(())
}

fn dbusmenu_prop_string(props: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let val = props.get(key)?;
    if let Ok(s) = String::try_from(val.clone()) {
        return Some(s);
    }
    if let Ok(s) = <&str>::try_from(val) {
        return Some(s.to_string());
    }
    None
}

fn dbusmenu_prop_bool(props: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    props.get(key).and_then(|v| bool::try_from(v).ok())
}

fn call_menu_event(conn: &Connection, service: &str, menu_path: &str, id: i32) -> zbus::Result<()> {
    let proxy = Proxy::new(conn, service, menu_path, DBUSMENU_IFACE)?;
    let _: () = proxy.call("Event", &(id, "clicked", Value::new(""), 0u32))?;
    let _: () = proxy.call("AboutToShow", &(id,))?;
    Ok(())
}

fn upsert_item(
    _core: &mut CoreCtx,
    wayland_systray: &mut WaylandSystray,
    item: WaylandSystrayItem,
) -> bool {
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
) -> Option<(Vec<u8>, i32, i32)> {
    let proxy = Proxy::new(conn, service, path, ITEM_IFACE).ok()?;

    let pixmaps: Vec<(i32, i32, Vec<u8>)> = proxy.get_property("IconPixmap").ok()?;
    if pixmaps.is_empty() {
        return None;
    }

    let mut best = None::<(i32, i32, Vec<u8>, i32)>;
    for (w, h, bytes) in pixmaps {
        if w <= 0 || h <= 0 {
            continue;
        }
        let score = (w * h).abs();
        match &best {
            Some((_, _, _, current)) if *current >= score => {}
            _ => best = Some((w, h, bytes, score)),
        }
    }
    let (w, h, bytes, _) = best?;
    let rgba = argb32_to_rgba(&bytes, w, h)?;
    Some((rgba, w, h))
}

fn argb32_to_rgba(bytes: &[u8], w: i32, h: i32) -> Option<Vec<u8>> {
    let px_count = (w as usize).checked_mul(h as usize)?;
    let need = px_count.checked_mul(4)?;
    if bytes.len() < need {
        return None;
    }

    let mut out = vec![0u8; need];
    for i in 0..px_count {
        let si = i * 4;
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
