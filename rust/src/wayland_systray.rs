use std::collections::HashSet;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::thread;

use zbus::blocking::{Connection, Proxy};

use crate::bar::paint::BarPainter;
use crate::contexts::CoreCtx;
use crate::types::{MouseButton, WaylandSystrayItem};

const WATCHER_SERVICE: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_IFACE: &str = "org.kde.StatusNotifierWatcher";

const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";

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
}

#[derive(Debug)]
enum SystrayEvt {
    ItemUpsert(WaylandSystrayItem),
    ItemRemoved(String, String),
}

pub struct WaylandSystrayRuntime {
    cmd_tx: Sender<SystrayCmd>,
    evt_rx: Receiver<SystrayEvt>,
}

impl WaylandSystrayRuntime {
    pub fn start() -> Option<Self> {
        let (cmd_tx, cmd_rx) = channel::<SystrayCmd>();
        let (evt_tx, evt_rx) = channel::<SystrayEvt>();

        let builder = thread::Builder::new().name("instantwm-wayland-systray".to_string());
        let spawn = builder.spawn(move || {
            run_systray_thread(cmd_rx, evt_tx);
        });

        if spawn.is_err() {
            return None;
        }

        Some(Self { cmd_tx, evt_rx })
    }

    pub fn poll_events(&self, core: &mut CoreCtx) -> bool {
        let mut changed = false;
        loop {
            match self.evt_rx.try_recv() {
                Ok(SystrayEvt::ItemUpsert(item)) => {
                    changed |= upsert_item(core, item);
                }
                Ok(SystrayEvt::ItemRemoved(service, path)) => {
                    let before = core.g.wayland_systray.items.len();
                    core.g
                        .wayland_systray
                        .items
                        .retain(|it| !(it.service == service && it.path == path));
                    changed |= core.g.wayland_systray.items.len() != before;
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
}

pub fn get_wayland_systray_width(core: &CoreCtx) -> i32 {
    if !core.g.cfg.showsystray {
        return 0;
    }
    let items = &core.g.wayland_systray.items;
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

pub fn hit_test_wayland_systray_item(
    core: &CoreCtx,
    mon: &crate::types::Monitor,
    local_x: i32,
) -> Option<usize> {
    let total_w = get_wayland_systray_width(core);
    if total_w <= 0 {
        return None;
    }
    let start_x = mon.work_rect.w - total_w;
    if local_x < start_x {
        return None;
    }

    let icon_h = core.g.cfg.bar_height.max(1);
    let spacing = core.g.cfg.systrayspacing.max(0);
    let mut x = start_x + spacing;
    for (idx, item) in core.g.wayland_systray.items.iter().enumerate() {
        let iw = scale_icon_width(item.icon_w, item.icon_h, icon_h);
        if local_x >= x && local_x < x + iw {
            return Some(idx);
        }
        x += iw + spacing;
    }
    None
}

pub fn draw_wayland_systray(
    core: &CoreCtx,
    mon: &crate::types::Monitor,
    painter: &mut crate::bar::wayland::WaylandBarPainter,
) {
    let total_w = get_wayland_systray_width(core);
    if total_w <= 0 {
        return;
    }

    if let Some(bg) = crate::bar::theme::status_scheme(core.g).map(|s| s.bg) {
        let bg_scheme = crate::bar::paint::BarScheme {
            fg: bg,
            bg,
            detail: bg,
        };
        painter.set_scheme(bg_scheme);
        painter.rect(
            mon.work_rect.w - total_w,
            0,
            total_w,
            core.g.cfg.bar_height,
            true,
            true,
        );
    }

    let icon_h = core.g.cfg.bar_height.max(1);
    let spacing = core.g.cfg.systrayspacing.max(0);
    let mut x = mon.work_rect.w - total_w + spacing;
    for item in &core.g.wayland_systray.items {
        if item.icon_w <= 0 || item.icon_h <= 0 {
            continue;
        }
        let draw_w = scale_icon_width(item.icon_w, item.icon_h, icon_h);
        let y = ((icon_h - icon_h.min(item.icon_h)) / 2).max(0);
        painter.blit_rgba_bgra(
            x,
            y,
            draw_w,
            icon_h,
            item.icon_w,
            item.icon_h,
            &item.icon_rgba,
        );
        x += draw_w + spacing;
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
            log::warn!("wayland systray: no session bus: {e}");
            return;
        }
    };

    register_watcher_host(&conn);
    let mut known_ids: HashSet<String> = HashSet::new();
    let _ = reconcile_items(&conn, &evt_tx, &mut known_ids);
    let mut ticks = 0u32;

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            dispatch_cmd(&conn, cmd);
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

fn dispatch_cmd(conn: &Connection, cmd: SystrayCmd) {
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
            let _ = call_item_method(conn, &service, &path, "ContextMenu", &(x, y));
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

fn upsert_item(core: &mut CoreCtx, item: WaylandSystrayItem) -> bool {
    if let Some(existing) = core
        .g
        .wayland_systray
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

    core.g.wayland_systray.items.push(item);
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
        let argb = u32::from_ne_bytes([bytes[si], bytes[si + 1], bytes[si + 2], bytes[si + 3]]);
        let a = ((argb >> 24) & 0xFF) as u8;
        let r = ((argb >> 16) & 0xFF) as u8;
        let g = ((argb >> 8) & 0xFF) as u8;
        let b = (argb & 0xFF) as u8;
        out[si] = r;
        out[si + 1] = g;
        out[si + 2] = b;
        out[si + 3] = a;
    }
    Some(out)
}
