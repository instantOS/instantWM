use std::sync::{Arc, Condvar, Mutex};

use crate::bar::MonitorHitCache;
use crate::bar::scene::{self, MonitorBarSnapshot};
use crate::core_state::WmCore;
use crate::types::MonitorId;

use super::buffer::RawBarBuffer;
use super::{BarRasterizer, WaylandBarRenderer};

pub(super) type Snapshots = Arc<Vec<MonitorBarSnapshot>>;

struct AsyncBarRenderResult {
    snapshots: Snapshots,
    buffers: Vec<(MonitorId, RawBarBuffer)>,
    hit_caches: Vec<(MonitorId, MonitorHitCache)>,
}

struct AsyncBarRenderShared {
    state: Mutex<WorkerState>,
    wake: Condvar,
    render_ping: Mutex<Option<smithay::reexports::calloop::ping::Ping>>,
}

#[derive(Default)]
struct WorkerState {
    request: Option<Snapshots>,
    result: Option<AsyncBarRenderResult>,
    stopped: bool,
}

impl AsyncBarRenderShared {
    fn next_request(&self) -> Option<Snapshots> {
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
    /// Snapshots most recently handed to the worker and not yet rendered.
    pending: Option<Snapshots>,
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
                    let result = render_snapshots(&mut painter, request);
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
            pending: None,
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

    pub(super) fn request(&mut self, snapshots: Vec<MonitorBarSnapshot>) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| **pending == snapshots)
        {
            return;
        }
        let snapshots = Arc::new(snapshots);
        self.shared.state.lock().unwrap().request = Some(Arc::clone(&snapshots));
        self.pending = Some(snapshots);
        self.shared.wake.notify_one();
    }

    /// Take a finished render if it still depicts `current`.
    fn take_result(&mut self, current: &[MonitorBarSnapshot]) -> Option<AsyncBarRenderResult> {
        let result = self.shared.take_result()?;
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| Arc::ptr_eq(pending, &result.snapshots))
        {
            self.pending = None;
        }
        // The scene may have changed, or reverted to the cached content,
        // while this render was in flight.
        (**result.snapshots == *current).then_some(result)
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

impl WaylandBarRenderer {
    pub(super) fn poll_result(&mut self, core: &mut WmCore, current: &[MonitorBarSnapshot]) {
        let Some(result) = self.async_runtime.take_result(current) else {
            return;
        };

        self.cached_buffers = result
            .buffers
            .iter()
            .map(|(monitor_id, buffer)| (*monitor_id, buffer.into()))
            .collect();
        self.cached_snapshots = result.snapshots;
        for (monitor_id, hit) in result.hit_caches {
            core.bar.replace_hit_cache(monitor_id, hit);
        }
    }
}

fn render_snapshots(painter: &mut BarRasterizer, snapshots: Snapshots) -> AsyncBarRenderResult {
    let mut buffers = Vec::new();
    let mut hit_caches = Vec::with_capacity(snapshots.len());

    for mon in snapshots.iter() {
        painter.set_fonts(&mon.fonts);
        painter.begin(mon.rect);
        let hit = scene::render_monitor_snapshot(mon, painter);

        if let Some(raw) = painter.finish_raw() {
            buffers.push((mon.monitor_id, raw));
        }
        hit_caches.push((mon.monitor_id, hit));
    }

    AsyncBarRenderResult {
        snapshots,
        buffers,
        hit_caches,
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
            pending: None,
        }
    }

    fn result(snapshots: &Snapshots) -> AsyncBarRenderResult {
        AsyncBarRenderResult {
            snapshots: Arc::clone(snapshots),
            buffers: Vec::new(),
            hit_caches: Vec::new(),
        }
    }

    fn snapshots(core: &WmCore) -> Vec<MonitorBarSnapshot> {
        scene::build_monitor_snapshots(core, 0, crate::bar::scene::BarSnapshotScope::Visible)
    }

    fn test_wm() -> crate::wm::WaylandWm {
        let mut wm = crate::wm::WaylandWm::new(crate::backend::WaylandBackendData::default());
        let mut monitor = crate::types::Monitor::new_with_values();
        monitor.set_available_rect(crate::types::Rect::new(0, 0, 800, 600));
        monitor.bar_height = 24;
        let id = wm.core.state.model.monitors.push(monitor);
        wm.core.state.model.monitors.set_selected(id);
        wm
    }

    /// Returns two distinct scenes.
    fn two_scenes() -> (Vec<MonitorBarSnapshot>, Vec<MonitorBarSnapshot>) {
        let mut wm = test_wm();
        let first = snapshots(&wm.core);
        wm.core.bar.set_status_text("changed");
        let second = snapshots(&wm.core);
        assert!(first != second);
        (first, second)
    }

    #[test]
    fn identical_scenes_are_requested_only_once() {
        let (first, _) = two_scenes();
        let mut runtime = runtime_without_worker();

        runtime.request(first.clone());
        let pending = Arc::clone(runtime.pending.as_ref().unwrap());
        runtime.request(first);

        assert!(Arc::ptr_eq(runtime.pending.as_ref().unwrap(), &pending));
    }

    #[test]
    fn reverting_to_cached_content_rejects_an_inflight_render() {
        let (cached, next) = two_scenes();
        let mut runtime = runtime_without_worker();
        runtime.request(next);
        let inflight = Arc::clone(runtime.pending.as_ref().unwrap());
        runtime.shared.publish_result(result(&inflight));

        assert!(runtime.take_result(&cached).is_none());
        // A later request for the same content must be allowed to render again.
        assert!(runtime.pending.is_none());
    }

    #[test]
    fn stale_result_does_not_clear_the_current_request() {
        let (first, second) = two_scenes();
        let mut runtime = runtime_without_worker();
        runtime.request(first);
        let stale = Arc::clone(runtime.pending.as_ref().unwrap());
        runtime.request(second.clone());
        let current = Arc::clone(runtime.pending.as_ref().unwrap());

        runtime.shared.publish_result(result(&stale));
        assert!(runtime.take_result(&second).is_none());
        assert!(runtime.pending.is_some());

        runtime.shared.publish_result(result(&current));
        assert!(runtime.take_result(&second).is_some());
        assert!(runtime.pending.is_none());
    }

    #[test]
    fn completed_results_are_latest_only() {
        let (first, second) = two_scenes();
        let runtime = runtime_without_worker();
        runtime.shared.publish_result(result(&Arc::new(first)));
        runtime
            .shared
            .publish_result(result(&Arc::new(second.clone())));

        assert!(**runtime.shared.take_result().unwrap().snapshots == second);
        assert!(runtime.shared.take_result().is_none());
    }

    #[test]
    fn fullscreen_keeps_a_ready_bar_without_drawing_it() {
        let mut wm = test_wm();
        let monitor = wm.core.state.model.selected_monitor_id();
        wm.core
            .state
            .model
            .monitor_mut(monitor)
            .unwrap()
            .bar_default_show = true;
        let mut client = crate::types::Client::new(crate::types::WindowId(91));
        client.set_tag_mask(wm.core.state.model.monitor(monitor).unwrap().visible_tags());
        let win =
            crate::test_support::add_selected_client(&mut wm.core.state.model, monitor, client);
        let _ = wm.core.state.model.set_fullscreen(win, true).unwrap();
        assert!(
            scene::build_monitor_snapshots(&wm.core, 0, scene::BarSnapshotScope::Visible)
                .is_empty()
        );
        let hidden =
            scene::build_monitor_snapshots(&wm.core, 0, scene::BarSnapshotScope::Configured);
        assert_eq!(hidden.len(), 1);

        let mut renderer = WaylandBarRenderer {
            cached_buffers: Vec::new(),
            cached_snapshots: Default::default(),
            async_runtime: runtime_without_worker(),
        };
        renderer.async_runtime.request(hidden.clone());
        let rendered = render_snapshots(&mut BarRasterizer::default(), Arc::new(hidden));
        renderer.async_runtime.shared.publish_result(rendered);
        assert!(super::super::render_bar_buffers(&mut wm.core, &mut renderer).is_empty());
        assert_eq!(renderer.cached_buffers.len(), 1);

        wm.core
            .state
            .model
            .monitor_mut(monitor)
            .unwrap()
            .set_selected_tags(crate::types::TagMask::single(2).unwrap());
        assert_eq!(
            super::super::render_bar_buffers(&mut wm.core, &mut renderer).len(),
            1
        );
        wm.core
            .state
            .model
            .monitor_mut(monitor)
            .unwrap()
            .set_selected_tags(crate::types::TagMask::single(1).unwrap());
        assert!(super::super::render_bar_buffers(&mut wm.core, &mut renderer).is_empty());

        // The first frame after leaving fullscreen uses the warm image while
        // updated title/tag styling is rasterized asynchronously.
        let _ = wm.core.state.model.set_fullscreen(win, false).unwrap();
        assert_eq!(
            super::super::render_bar_buffers(&mut wm.core, &mut renderer).len(),
            1
        );
        wm.core
            .state
            .model
            .monitor_mut(monitor)
            .unwrap()
            .bar_default_show = false;
        assert!(super::super::render_bar_buffers(&mut wm.core, &mut renderer).is_empty());
        // Another output at the same coordinates must not make the hidden
        // monitor's cached buffer visible.
        let mut other = crate::types::Monitor::new_with_values();
        other.set_available_rect(crate::types::Rect::new(0, 0, 800, 600));
        other.bar_height = 24;
        other.bar_default_show = true;
        wm.core.state.model.monitors.push(other);
        assert!(super::super::render_bar_buffers(&mut wm.core, &mut renderer).is_empty());
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
        shared.state.lock().unwrap().request = Some(Arc::new(Vec::new()));

        drop(runtime);
        assert!(shared.next_request().is_none());
        assert!(shared.state.lock().unwrap().request.is_none());
    }
}
