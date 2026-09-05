use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

use smithay::utils::Scale;

use crate::bar::scene;
use crate::contexts::CoreCtx;

use super::WaylandBarPainter;
use super::buffer::RawBarBuffer;

#[derive(Clone)]
struct AsyncBarRenderRequest {
    generation: u64,
    content_key: u64,
    monitors: Vec<scene::MonitorBarSnapshot>,
}

struct AsyncBarRenderResult {
    generation: u64,
    content_key: u64,
    buffers: Vec<RawBarBuffer>,
    monitor_updates: Vec<scene::MonitorRenderOutputWithId>,
}

struct AsyncBarRenderShared {
    pending: Mutex<PendingRender>,
    wake: Condvar,
    results_tx: Sender<AsyncBarRenderResult>,
    render_ping: Mutex<Option<smithay::reexports::calloop::ping::Ping>>,
}

#[derive(Default)]
struct PendingRender {
    request: Option<AsyncBarRenderRequest>,
    stopped: bool,
}

impl AsyncBarRenderShared {
    fn next_request(&self) -> Option<AsyncBarRenderRequest> {
        let mut pending = self.pending.lock().unwrap();
        loop {
            if pending.stopped {
                return None;
            }
            if let Some(request) = pending.request.take() {
                return Some(request);
            }
            pending = self.wake.wait(pending).unwrap();
        }
    }
}

pub(super) struct AsyncBarRenderRuntime {
    shared: Arc<AsyncBarRenderShared>,
    results_rx: Receiver<AsyncBarRenderResult>,
    pending_content_key: Option<u64>,
    pending_generation: u64,
    next_generation: u64,
}

impl AsyncBarRenderRuntime {
    pub(super) fn spawn() -> Self {
        let (results_tx, results_rx) = mpsc::channel();
        let shared = Arc::new(AsyncBarRenderShared {
            pending: Mutex::new(PendingRender::default()),
            wake: Condvar::new(),
            results_tx,
            render_ping: Mutex::new(None),
        });

        let worker_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("instantwm-wayland-bar".to_string())
            .spawn(move || {
                let mut painter = WaylandBarPainter::new_worker_painter();
                while let Some(request) = worker_shared.next_request() {
                    let result = render_snapshot(&mut painter, request);
                    if worker_shared.results_tx.send(result).is_err() {
                        break;
                    }
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
            pending_content_key: None,
            pending_generation: 0,
            next_generation: 0,
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

    fn take_result(&mut self, content_key: u64) -> Option<AsyncBarRenderResult> {
        let mut latest = None;
        while let Ok(result) = self.results_rx.try_recv() {
            if !is_current_generation(result.generation, self.pending_generation) {
                continue;
            }
            self.pending_content_key = None;
            // The scene may have reverted to its cached content while this
            // render was in flight, without scheduling another generation.
            if result.content_key == content_key {
                latest = Some(result);
            }
        }
        latest
    }
}

impl Drop for AsyncBarRenderRuntime {
    fn drop(&mut self) {
        let mut pending = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        pending.stopped = true;
        pending.request = None;
        self.shared.wake.notify_one();
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
    if runtime.pending_content_key == Some(key) {
        return;
    }

    runtime.next_generation = runtime.next_generation.wrapping_add(1).max(1);
    let generation = runtime.next_generation;
    let mut pending = runtime.shared.pending.lock().unwrap();
    pending.request = Some(AsyncBarRenderRequest {
        generation,
        content_key: key,
        monitors,
    });
    runtime.pending_content_key = Some(key);
    runtime.pending_generation = generation;
    runtime.shared.wake.notify_one();
}

pub(super) fn poll_result(core: &mut CoreCtx, painter: &mut WaylandBarPainter, key: u64) {
    let Some(runtime) = painter.async_runtime.as_mut() else {
        return;
    };

    let Some(result) = runtime.take_result(key) else {
        return;
    };

    painter.cached_buffers = result.buffers.iter().map(|b| b.into()).collect();
    painter.cached_key = result.content_key;

    for update in result.monitor_updates {
        core.bar
            .replace_hit_cache(update.monitor_id, update.output.hit_cache);
        if let Some(mon) = core.model_mut().monitor_mut(update.monitor_id) {
            mon.bar_clients_width = update.output.bar_clients_width;
        }
    }
}

fn is_current_generation(result: u64, pending: u64) -> bool {
    result == pending
}

fn render_snapshot(
    painter: &mut WaylandBarPainter,
    request: AsyncBarRenderRequest,
) -> AsyncBarRenderResult {
    let mut buffers = Vec::new();
    let mut monitor_updates = Vec::new();

    for mut mon in request.monitors {
        if mon.is_selected_monitor {
            mon.presentation.status.ensure_items_parsed();
        }

        painter.set_fonts(&mon.fonts);
        painter.begin(Scale::from(1.0), mon.rect);
        let output = scene::render_monitor_snapshot(&mon, painter);

        if let Some(raw) = painter.finish_raw() {
            buffers.push(raw);
        }
        monitor_updates.push(scene::MonitorRenderOutputWithId {
            monitor_id: mon.monitor_id,
            output,
        });
    }

    AsyncBarRenderResult {
        generation: request.generation,
        content_key: request.content_key,
        buffers,
        monitor_updates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_without_worker() -> AsyncBarRenderRuntime {
        let (results_tx, results_rx) = mpsc::channel();
        AsyncBarRenderRuntime {
            shared: Arc::new(AsyncBarRenderShared {
                pending: Mutex::new(PendingRender::default()),
                wake: Condvar::new(),
                results_tx,
                render_ping: Mutex::new(None),
            }),
            results_rx,
            pending_content_key: Some(20),
            pending_generation: 2,
            next_generation: 2,
        }
    }

    fn result(generation: u64, content_key: u64) -> AsyncBarRenderResult {
        AsyncBarRenderResult {
            generation,
            content_key,
            buffers: Vec::new(),
            monitor_updates: Vec::new(),
        }
    }

    #[test]
    fn reverting_to_cached_content_rejects_an_inflight_render() {
        let mut runtime = runtime_without_worker();
        runtime.shared.results_tx.send(result(2, 20)).unwrap();

        assert!(runtime.take_result(10).is_none());
        // A later request for 20 must be allowed to render again.
        assert_eq!(runtime.pending_content_key, None);
    }

    #[test]
    fn stale_result_does_not_clear_the_current_request() {
        let mut runtime = runtime_without_worker();
        runtime.shared.results_tx.send(result(1, 10)).unwrap();
        assert!(runtime.take_result(20).is_none());
        assert_eq!(runtime.pending_content_key, Some(20));

        runtime.shared.results_tx.send(result(2, 20)).unwrap();
        assert_eq!(runtime.take_result(20).unwrap().content_key, 20);
        assert_eq!(runtime.pending_content_key, None);
    }

    #[test]
    fn dropping_runtime_wakes_and_stops_an_idle_worker() {
        let runtime = runtime_without_worker();
        let shared = Arc::clone(&runtime.shared);
        let (finished_tx, finished_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            assert!(shared.next_request().is_none());
            finished_tx.send(()).unwrap();
        });

        drop(runtime);
        finished_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("bar worker did not stop");
        worker.join().unwrap();
    }

    #[test]
    fn dropping_runtime_discards_pending_work() {
        let runtime = runtime_without_worker();
        let shared = Arc::clone(&runtime.shared);
        shared.pending.lock().unwrap().request = Some(AsyncBarRenderRequest {
            generation: 2,
            content_key: 20,
            monitors: Vec::new(),
        });

        drop(runtime);
        assert!(shared.next_request().is_none());
        assert!(shared.pending.lock().unwrap().request.is_none());
    }

    #[test]
    fn only_the_exact_pending_generation_can_replace_bar_buffers() {
        assert!(!is_current_generation(4, 5));
        assert!(is_current_generation(5, 5));
        assert!(!is_current_generation(6, 5));
    }
}
