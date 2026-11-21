//! DRM session runtime: output state, event sources, and the calloop tick.
//!
//! `WaylandState` is intentionally not owned here. Calloop uses it as loop
//! data (`&mut`), while the session state is captured by the loop closure.
//! Keeping them separate avoids a self-borrow between the loop data and the
//! resources the tick needs.

use smithay::reexports::calloop::{EventLoop, LoopHandle};
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::utils::{Clock, Monotonic};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::mem;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Instant;

use super::super::engine;
use super::DrmRuntimeEvent;
use super::devices::{DrmDevices, PRIMARY_DEVICE};
use super::frames::output_refresh;
use super::layout::{DrmLayoutState, refresh_drm_layout_state};
use super::scheduling::DrmLoopState;
use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::render::cursor_theme::CursorManager;
use crate::backend::wayland::render::drm::{OutputSurfaceEntry, ScanoutId};

/// Output state owned by the DRM session.
///
/// These fields are mutated by topology reconciliation, output transactions,
/// rendering, and frame scheduling. Keeping them together lets those stages
/// be methods with narrow inputs instead of free functions with six to eight
/// parameters.
pub(super) struct DrmOutputRuntime {
    pub(super) layout_state: DrmLayoutState,
    pub(super) input_dimensions: Rc<Cell<crate::types::Rect>>,
    pub(super) loop_state: DrmLoopState,
    pub(super) output_surfaces: Vec<OutputSurfaceEntry>,
    pub(super) devices: Rc<RefCell<DrmDevices>>,
    pub(super) render_failures: HashMap<ScanoutId, u32>,
    pub(super) cursor_manager: CursorManager,
    pub(super) start_time: Instant,
}

/// Long-lived session services and channels captured by the DRM loop closure.
pub(super) struct DrmSession {
    outputs: DrmOutputRuntime,
    runtime_event_tx: mpsc::Sender<DrmRuntimeEvent>,
    runtime_event_rx: mpsc::Receiver<DrmRuntimeEvent>,
    led_state_rx: mpsc::Receiver<smithay::input::keyboard::LedState>,
    ipc_server: Option<crate::ipc::IpcServer>,
    monotonic_clock: Clock<Monotonic>,
    render_retry_guard: crate::runtime::AnimationTimerGuard,
}

impl DrmSession {
    pub(super) fn new(
        outputs: DrmOutputRuntime,
        runtime_event_tx: mpsc::Sender<DrmRuntimeEvent>,
        runtime_event_rx: mpsc::Receiver<DrmRuntimeEvent>,
        led_state_rx: mpsc::Receiver<smithay::input::keyboard::LedState>,
        ipc_server: Option<crate::ipc::IpcServer>,
    ) -> Self {
        Self {
            outputs,
            runtime_event_tx,
            runtime_event_rx,
            led_state_rx,
            ipc_server,
            monotonic_clock: Clock::new(),
            render_retry_guard: crate::runtime::AnimationTimerGuard::new(),
        }
    }

    /// Run the DRM event loop until a fatal session condition stops it.
    pub(super) fn run(mut self, mut event_loop: EventLoop<WaylandState>, state: &mut WaylandState) {
        let loop_signal = event_loop.get_signal();
        let loop_handle = event_loop.handle();

        event_loop
            .run(None, state, move |state| {
                if self.tick(state, &loop_handle) {
                    loop_signal.stop();
                }
            })
            .expect("event loop run");
    }

    /// One compositor tick. Returns `true` when the loop must stop.
    fn tick(
        &mut self,
        state: &mut WaylandState,
        loop_handle: &LoopHandle<'_, WaylandState>,
    ) -> bool {
        state.dispatch_pending_commits();

        let (pointer_moved, topology_changed) = self
            .outputs
            .drain_runtime_events(&self.runtime_event_rx, &self.monotonic_clock);
        if self.outputs.primary_device_removed() {
            log::error!("Primary rendering GPU was removed; stopping the DRM session");
            return true;
        }

        if self
            .outputs
            .loop_state
            .take_topology_change(topology_changed)
            && self
                .outputs
                .reconcile_outputs(state, loop_handle, &self.runtime_event_tx)
        {
            self.outputs.after_topology_change();
        }

        self.outputs.process_frame_callbacks(state, loop_handle);
        self.outputs.schedule_commit_timing(state, loop_handle);
        state.dispatch_pending_commits();

        engine::event_loop_tick_and_request_render(state, &mut self.ipc_server);
        if mem::take(&mut state.native.runtime.lid_policy_dirty) {
            state
                .native
                .queue_output_policy_projection(&state.wm.core.state.config.monitors);
        }

        if self.outputs.loop_state.session_active {
            let graphics = state.graphics.as_mut().expect("DRM graphics initialized");
            graphics.with_renderer(|renderer| {
                self.outputs
                    .process_output_configurations(&mut state.native, renderer)
            });
        }

        if state.native.project_completed_output_transactions() {
            crate::monitor::refresh_monitor_layout(&mut state.ctx());
            state.native.push_command(
                crate::backend::wayland::commands::WmCommand::SyncLayerExclusiveZones,
            );
            self.outputs.refresh_layout(state);
        }

        state.native.project_completed_output_power_requests();
        if self.outputs.loop_state.session_active {
            self.outputs
                .process_output_power_requests(&mut state.native);
        }
        state.native.project_completed_output_power_requests();

        if pointer_moved {
            self.outputs.mark_pointer_output_dirty(state);
        }

        engine::process_animations_and_request_render(state);
        self.outputs.process_commit_redraws(state);
        self.outputs.mark_bar_updates_dirty(state);

        if state.wm.core.work.input_config {
            state.wm.core.work.input_config = false;
            crate::backend::wayland::input::drm::reconfigure_all_devices(
                &mut state.native.runtime.tracked_devices,
                &state.wm.core.state.config.input,
            );
        }

        if state.wm.core.work.cursor_config {
            state.wm.core.work.cursor_config = false;
            self.outputs.reload_cursor(state);
        }

        self.process_led_states(state);
        self.outputs.update_cursor_animation(state);

        if let Some(keyboard_handle) = state.native.seat.get_keyboard() {
            let pointer_handle = state.native.pointer.clone();
            self.outputs
                .process_cursor_warp(state, &pointer_handle, &keyboard_handle);
        }

        self.outputs.render(state, loop_handle);

        // Retry persistent render failures at a bounded cadence. Dirty
        // outputs with a page flip in flight are woken by DRM vblank.
        self.render_retry_guard.ensure_armed(
            self.outputs.loop_state.has_renderable_dirty_outputs(),
            loop_handle,
            |_| false,
        );

        state.dispatch_pending_commits();

        state.native.display_handle.flush_clients().is_err()
    }

    fn process_led_states(&mut self, state: &mut WaylandState) {
        while let Ok(led_state) = self.led_state_rx.try_recv() {
            let leds = smithay::reexports::input::Led::from(led_state);
            for device in state.native.runtime.tracked_devices.iter_mut() {
                use smithay::reexports::input::DeviceCapability;
                if device.has_capability(DeviceCapability::Keyboard) {
                    device.led_update(leds);
                }
            }
        }
    }
}

impl DrmOutputRuntime {
    fn drain_runtime_events(
        &mut self,
        runtime_event_rx: &mpsc::Receiver<DrmRuntimeEvent>,
        monotonic_clock: &Clock<Monotonic>,
    ) -> (bool, bool) {
        process_runtime_events(
            runtime_event_rx,
            &mut self.loop_state,
            &self.layout_state,
            &mut self.output_surfaces,
            monotonic_clock,
        )
    }

    fn primary_device_removed(&self) -> bool {
        self.loop_state
            .removed_devices
            .contains(&self.devices.borrow().devices[&PRIMARY_DEVICE].devnum)
    }

    fn after_topology_change(&mut self) {
        self.loop_state.mark_all_dirty();
        self.render_failures.retain(|scanout, _| {
            self.output_surfaces
                .iter()
                .any(|entry| entry.scanout() == *scanout)
        });
    }

    fn refresh_layout(&mut self, state: &mut WaylandState) {
        refresh_drm_layout_state(state, &mut self.output_surfaces, &mut self.layout_state);
        self.input_dimensions.set(self.layout_state.layout);
        crate::backend::wayland::input::recover_pointer_after_output_change(state);
    }

    pub(super) fn mark_pointer_output_dirty(&mut self, state: &WaylandState) {
        self.loop_state.mark_pointer_output_dirty(
            crate::types::Point::new(
                state.native.runtime.pointer_location.x as i32,
                state.native.runtime.pointer_location.y as i32,
            ),
            &self.layout_state,
        );
    }

    fn mark_bar_updates_dirty(&mut self, state: &WaylandState) {
        let bar_update_seq = state.wm.core.bar.update_seq();
        if self.loop_state.last_bar_update_seq != bar_update_seq {
            self.loop_state.last_bar_update_seq = bar_update_seq;
            self.loop_state.mark_all_dirty();
        }
    }
}

/// Drain queued DRM runtime events into the loop state.
///
/// This stays a free function because the unit tests drive it with a minimal
/// `DrmLoopState`, layout, and surface list rather than a full session.
pub(super) fn process_runtime_events(
    runtime_event_rx: &mpsc::Receiver<DrmRuntimeEvent>,
    loop_state: &mut DrmLoopState,
    layout_state: &DrmLayoutState,
    output_surfaces: &mut [OutputSurfaceEntry],
    monotonic_clock: &Clock<Monotonic>,
) -> (bool, bool) {
    let mut pointer_moved = false;
    let mut topology_changed = false;
    while let Ok(event) = runtime_event_rx.try_recv() {
        match event {
            DrmRuntimeEvent::SessionPaused => {
                loop_state.session_active = false;
            }
            DrmRuntimeEvent::SessionActivated => {
                // Keep in-flight flips intact: Smithay retains their buffers
                // across activate(), and only a real vblank may retire them.
                loop_state.session_active = true;
                loop_state.mark_all_dirty();
                // The dock may have disappeared while this session was inactive.
                topology_changed = true;
            }
            DrmRuntimeEvent::VBlank(crtc) => {
                if !loop_state.render_flags.contains_key(&crtc) {
                    continue;
                }
                loop_state
                    .presentation_scheduler
                    .presentation_completed(crtc, Instant::now());
                if let Some(entry) = output_surfaces
                    .iter_mut()
                    .find(|entry| entry.scanout() == crtc)
                {
                    let Some(surface) = entry.surface.as_mut() else {
                        loop_state.pending_crtcs.remove(&crtc);
                        continue;
                    };
                    match surface.frame_submitted() {
                        Ok(Some(mut metadata)) => {
                            let seq = loop_state.presentation_seq.entry(crtc).or_insert(0);
                            *seq += 1;
                            metadata.presentation_feedback.presented(
                                monotonic_clock.now(),
                                output_refresh(entry),
                                *seq,
                                wp_presentation_feedback::Kind::Vsync,
                            );
                        }
                        Ok(None) => {}
                        Err(err) => {
                            log::warn!("frame_submitted failed for {:?}: {err}", crtc);
                        }
                    }
                }
                loop_state.pending_crtcs.remove(&crtc);
            }
            DrmRuntimeEvent::PointerMoved { old_location } => {
                loop_state.mark_pointer_output_dirty(old_location, layout_state);
                pointer_moved = true;
            }
            DrmRuntimeEvent::OutputTopologyChanged => topology_changed = true,
            DrmRuntimeEvent::DeviceRemoved(devnum) => {
                loop_state.removed_devices.insert(devnum);
                topology_changed = true;
            }
        }
    }
    (pointer_moved, topology_changed)
}
