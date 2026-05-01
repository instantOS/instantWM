use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};

use smithay::utils::Scale;

use crate::bar::scene;
use crate::contexts::CoreCtx;

use super::WaylandBarPainter;
use super::buffer::{RawBarBuffer, raw_to_bar_buffer};
use super::systray;

#[derive(Clone)]
struct AsyncBarRenderRequest {
    key: u64,
    monitors: Vec<scene::MonitorBarSnapshot>,
}

struct AsyncBarRenderResult {
    key: u64,
    buffers: Vec<RawBarBuffer>,
    monitor_updates: Vec<scene::MonitorRenderOutputWithId>,
}

struct AsyncBarRenderShared {
    pending: Mutex<Option<AsyncBarRenderRequest>>,
    wake: Condvar,
    results_tx: Sender<AsyncBarRenderResult>,
    render_ping: Mutex<Option<smithay::reexports::calloop::ping::Ping>>,
}

pub(super) struct AsyncBarRenderRuntime {
    shared: Arc<AsyncBarRenderShared>,
    results_rx: Receiver<AsyncBarRenderResult>,
    pending_key: u64,
}

impl AsyncBarRenderRuntime {
    pub(super) fn spawn() -> Self {
        let (results_tx, results_rx) = mpsc::channel();
        let shared = Arc::new(AsyncBarRenderShared {
            pending: Mutex::new(None),
            wake: Condvar::new(),
            results_tx,
            render_ping: Mutex::new(None),
        });

        let worker_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("instantwm-wayland-bar".to_string())
            .spawn(move || {
                let mut painter = WaylandBarPainter::new_worker_painter();
                loop {
                    let request = {
                        let mut guard = worker_shared.pending.lock().unwrap();
                        loop {
                            if let Some(request) = guard.take() {
                                break request;
                            }
                            guard = worker_shared.wake.wait(guard).unwrap();
                        }
                    };

                    let result = render_snapshot(&mut painter, request);
                    let _ = worker_shared.results_tx.send(result);
                    if let Ok(guard) = worker_shared.render_ping.lock()
                        && let Some(ping) = guard.as_ref()
                    {
                        ping.ping();
                    }
                }
            })
            .expect("failed to spawn Wayland bar worker");

        Self {
            shared,
            results_rx,
            pending_key: 0,
        }
    }

    pub(super) fn set_render_ping(
        &mut self,
        render_ping: Option<smithay::reexports::calloop::ping::Ping>,
    ) {
        if let Ok(mut guard) = self.shared.render_ping.lock() {
            *guard = render_ping;
        }
    }
}

pub(super) fn request_render(
    painter: &mut WaylandBarPainter,
    key: u64,
    monitors: Vec<scene::MonitorBarSnapshot>,
) {
    let Some(runtime) = painter.async_runtime.as_mut() else {
        return;
    };
    if runtime.pending_key == key {
        return;
    }

    let mut pending = runtime.shared.pending.lock().unwrap();
    *pending = Some(AsyncBarRenderRequest { key, monitors });
    runtime.pending_key = key;
    runtime.shared.wake.notify_one();
}

pub(super) fn poll_result(core: &mut CoreCtx, painter: &mut WaylandBarPainter) {
    let Some(runtime) = painter.async_runtime.as_mut() else {
        return;
    };

    let mut latest = None;
    loop {
        match runtime.results_rx.try_recv() {
            Ok(result) => {
                if result.key < runtime.pending_key {
                    continue;
                }
                latest = Some(result);
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
        }
    }

    let Some(result) = latest else {
        return;
    };

    painter.cached_buffers = result.buffers.iter().map(raw_to_bar_buffer).collect();
    painter.cached_key = result.key;

    for update in result.monitor_updates {
        core.bar
            .replace_hit_cache(update.monitor_id, update.output.hit_cache);
        if let Some(mon) = core.globals_mut().monitor_mut(update.monitor_id) {
            mon.bar_clients_width = update.output.bar_clients_width;
            mon.activeoffset = update.output.activeoffset;
        }
    }
}

fn render_snapshot(
    painter: &mut WaylandBarPainter,
    request: AsyncBarRenderRequest,
) -> AsyncBarRenderResult {
    let mut buffers = Vec::new();
    let mut monitor_updates = Vec::new();

    for mut mon in request.monitors {
        if mon.is_selected_monitor
            && mon.status_items.is_empty()
            && let Some(text) = mon.status_text.as_deref()
        {
            mon.status_items = crate::bar::status::parse_status(text.as_bytes()).items;
        }

        painter.set_font_size(mon.font_size);
        painter.begin(
            Scale::from(1.0),
            mon.rect.x,
            mon.rect.y,
            mon.rect.w,
            mon.rect.h,
        );
        let output = scene::render_monitor_snapshot(&mon, painter);
        let bar_height = mon.rect.h;
        let tray_layout = mon
            .systray
            .as_ref()
            .map(|s| scene::worker_systray_layout(s, mon.rect.w, bar_height.max(1)));
        if let (Some(systray), Some(layout)) = (&mon.systray, &tray_layout) {
            systray::draw_snapshot(painter, systray, layout, bar_height);
        }

        if let Some(raw) = painter.finish_raw() {
            buffers.push(raw);
        }
        monitor_updates.push(scene::MonitorRenderOutputWithId {
            monitor_id: mon.monitor_id,
            output,
        });
    }

    AsyncBarRenderResult {
        key: request.key,
        buffers,
        monitor_updates,
    }
}
