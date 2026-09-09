use std::sync::{Arc, Condvar, Mutex};

use smithay::utils::Scale;

use crate::bar::scene;
use crate::contexts::CoreCtx;

use super::buffer::RawBarBuffer;
use super::{BarRasterizer, WaylandBarRenderer};

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
    state: Mutex<WorkerState>,
    wake: Condvar,
    render_ping: Mutex<Option<smithay::reexports::calloop::ping::Ping>>,
}

#[derive(Default)]
struct WorkerState {
    request: Option<AsyncBarRenderRequest>,
    result: Option<AsyncBarRenderResult>,
    stopped: bool,
}

impl AsyncBarRenderShared {
    fn next_request(&self) -> Option<AsyncBarRenderRequest> {
        let mut state = self.state.lock().unwrap();
        loop {
            if state.stopped {
                return None;
            }
            if let Some(request) = state.request.take() {
                return Some(request);
            }
            state = self.wake.wait(state).unwrap();
        }
    }

    fn publish_result(&self, result: AsyncBarRenderResult) {
        // A completed bar image is only useful until a newer one exists. An
        // unbounded queue here turns status bursts into full-width pixel-buffer
        // retention and can leave a large allocator high-water mark.
        self.state.lock().unwrap().result = Some(result);
    }

    fn take_result(&self) -> Option<AsyncBarRenderResult> {
        self.state.lock().unwrap().result.take()
    }
}

pub(super) struct AsyncBarRenderRuntime {
    shared: Arc<AsyncBarRenderShared>,
    pending_content_key: Option<u64>,
    pending_generation: u64,
    next_generation: u64,
}

impl AsyncBarRenderRuntime {
    pub(super) fn spawn() -> Self {
        let shared = Arc::new(AsyncBarRenderShared {
            state: Mutex::new(WorkerState::default()),
            wake: Condvar::new(),
            render_ping: Mutex::new(None),
        });

        let worker_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("instantwm-wayland-bar".to_string())
            .spawn(move || {
                let mut painter = BarRasterizer::default();
                while let Some(request) = worker_shared.next_request() {
                    let result = render_snapshot(&mut painter, request);
                    worker_shared.publish_result(result);
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
        let result = self.shared.take_result()?;
        if !is_current_generation(result.generation, self.pending_generation) {
            return None;
        }
        self.pending_content_key = None;
        // The scene may have reverted to its cached content while this render
        // was in flight, without scheduling another generation.
        (result.content_key == content_key).then_some(result)
    }
}

impl Drop for AsyncBarRenderRuntime {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.stopped = true;
        state.request = None;
        state.result = None;
        self.shared.wake.notify_one();
    }
}

pub(super) fn request_render(
    renderer: &mut WaylandBarRenderer,
    key: u64,
    monitors: Vec<scene::MonitorBarSnapshot>,
) {
    let runtime = &mut renderer.async_runtime;
    if runtime.pending_content_key == Some(key) {
        return;
    }

    runtime.next_generation = runtime.next_generation.wrapping_add(1).max(1);
    let generation = runtime.next_generation;
    let mut state = runtime.shared.state.lock().unwrap();
    state.request = Some(AsyncBarRenderRequest {
        generation,
        content_key: key,
        monitors,
    });
    runtime.pending_content_key = Some(key);
    runtime.pending_generation = generation;
    runtime.shared.wake.notify_one();
}

pub(super) fn poll_result(core: &mut CoreCtx, renderer: &mut WaylandBarRenderer, key: u64) {
    let Some(result) = renderer.async_runtime.take_result(key) else {
        return;
    };

    renderer.cached_buffers = result.buffers.iter().map(|b| b.into()).collect();
    renderer.cached_key = result.content_key;

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
    painter: &mut BarRasterizer,
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
    use std::sync::mpsc;

    fn runtime_without_worker() -> AsyncBarRenderRuntime {
        AsyncBarRenderRuntime {
            shared: Arc::new(AsyncBarRenderShared {
                state: Mutex::new(WorkerState::default()),
                wake: Condvar::new(),
                render_ping: Mutex::new(None),
            }),
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
        runtime.shared.publish_result(result(2, 20));

        assert!(runtime.take_result(10).is_none());
        // A later request for 20 must be allowed to render again.
        assert_eq!(runtime.pending_content_key, None);
    }

    #[test]
    fn stale_result_does_not_clear_the_current_request() {
        let mut runtime = runtime_without_worker();
        runtime.shared.publish_result(result(1, 10));
        assert!(runtime.take_result(20).is_none());
        assert_eq!(runtime.pending_content_key, Some(20));

        runtime.shared.publish_result(result(2, 20));
        assert_eq!(runtime.take_result(20).unwrap().content_key, 20);
        assert_eq!(runtime.pending_content_key, None);
    }

    #[test]
    fn completed_results_are_latest_only() {
        let mut runtime = runtime_without_worker();
        runtime.shared.publish_result(result(1, 10));
        runtime.shared.publish_result(result(2, 20));

        assert_eq!(runtime.take_result(20).unwrap().content_key, 20);
        assert!(runtime.shared.take_result().is_none());
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
        shared.state.lock().unwrap().request = Some(AsyncBarRenderRequest {
            generation: 2,
            content_key: 20,
            monitors: Vec::new(),
        });

        drop(runtime);
        assert!(shared.next_request().is_none());
        assert!(shared.state.lock().unwrap().request.is_none());
    }

    #[test]
    fn only_the_exact_pending_generation_can_replace_bar_buffers() {
        assert!(!is_current_generation(4, 5));
        assert!(is_current_generation(5, 5));
        assert!(!is_current_generation(6, 5));
    }
}
