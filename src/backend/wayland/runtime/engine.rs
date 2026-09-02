//! Shared per-tick scheduling for the nested and DRM/KMS runtimes.
//!
//! Backend startup lives in [`super::bootstrap`], and queued compositor
//! commands are applied by [`super::dispatch`].

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::backend::wayland::compositor::WaylandState;
use crate::wm::Wm;
use smithay::output::Output;
use smithay::reexports::calloop::LoopHandle;
use smithay::utils::{Clock, Monotonic, Time};

/// Per-output presentation wakeups shared by nested and DRM runtimes.
///
/// Callback-only commits and client commit-timing deadlines use independent
/// generations, while sharing refresh prediction and the compositor clock.
#[derive(Debug)]
pub(crate) struct PresentationScheduler<K> {
    callbacks: Rc<RefCell<HashMap<K, u64>>>,
    commit_timing: Rc<RefCell<HashMap<K, (u64, Instant)>>>,
    presentation_phase: Rc<RefCell<HashMap<K, Instant>>>,
    next_generation: Cell<u64>,
}

impl<K> Default for PresentationScheduler<K> {
    fn default() -> Self {
        Self {
            callbacks: Rc::new(RefCell::new(HashMap::new())),
            commit_timing: Rc::new(RefCell::new(HashMap::new())),
            presentation_phase: Rc::new(RefCell::new(HashMap::new())),
            next_generation: Cell::new(0),
        }
    }
}

impl<K> PresentationScheduler<K>
where
    K: Clone + Debug + Eq + Hash + 'static,
{
    pub(crate) fn arm_callbacks(
        &self,
        key: K,
        loop_handle: &LoopHandle<'_, WaylandState>,
        output: &Output,
        start_time: Instant,
    ) {
        if self.callbacks.borrow().contains_key(&key) {
            return;
        }

        let generation = self.next_generation.get().wrapping_add(1);
        self.next_generation.set(generation);
        self.callbacks.borrow_mut().insert(key.clone(), generation);

        let output = output.clone();
        let period = output_frame_callback_delay(&output);
        let delay = self.next_presentation_delay(&key, period, Instant::now());
        let armed_for_timer = Rc::clone(&self.callbacks);
        let timer_key = key.clone();
        if let Err(err) = loop_handle.insert_source(
            smithay::reexports::calloop::timer::Timer::from_duration(delay),
            move |_, _, state| {
                let is_current = armed_for_timer
                    .borrow()
                    .get(&timer_key)
                    .is_some_and(|current| *current == generation);
                if is_current {
                    armed_for_timer.borrow_mut().remove(&timer_key);
                    crate::backend::wayland::render::frame::release_fifo_barriers(state, &output);
                    crate::backend::wayland::render::frame::send_frame_callbacks(
                        state,
                        &output,
                        start_time.elapsed(),
                    );
                }
                smithay::reexports::calloop::timer::TimeoutAction::Drop
            },
        ) {
            let is_current = self
                .callbacks
                .borrow()
                .get(&key)
                .is_some_and(|current| *current == generation);
            if is_current {
                self.callbacks.borrow_mut().remove(&key);
            }
            log::warn!("failed to arm frame-callback timer for {key:?}: {err}");
        }
    }

    pub(crate) fn presentation_submitted(&self, key: &K) {
        self.callbacks.borrow_mut().remove(key);
    }

    /// Record the observed completion phase used to align timer-backed work
    /// with the output rather than starting a free-running clock at request
    /// time.
    pub(crate) fn presentation_completed(&self, key: K, now: Instant) {
        self.presentation_phase.borrow_mut().insert(key, now);
    }

    pub(crate) fn remove_output(&self, key: &K) {
        self.callbacks.borrow_mut().remove(key);
        self.commit_timing.borrow_mut().remove(key);
        self.presentation_phase.borrow_mut().remove(key);
    }

    /// Service eligible commit timestamps and arm a wakeup one refresh before
    /// the earliest remaining deadline so rendering can target that refresh.
    pub(crate) fn schedule_commit_timing(
        &self,
        key: K,
        loop_handle: &LoopHandle<'_, WaylandState>,
        state: &mut WaylandState,
        output: &Output,
    ) {
        let period = output_frame_callback_delay(output);
        let clock = Clock::<Monotonic>::new();
        let presentation_delay = self.next_presentation_delay(&key, period, Instant::now());
        let frame_target = clock.now() + presentation_delay;
        let Some(deadline) = crate::backend::wayland::render::frame::service_commit_timing(
            state,
            output,
            frame_target,
        ) else {
            self.commit_timing.borrow_mut().remove(&key);
            return;
        };
        let deadline: Time<Monotonic> = deadline.into();
        let delay = commit_timing_wake_delay(
            Duration::from(deadline),
            Duration::from(clock.now()),
            period,
        );
        let wake_at = Instant::now() + delay;
        if self
            .commit_timing
            .borrow()
            .get(&key)
            .is_some_and(|(_, current)| *current <= wake_at)
        {
            return;
        }

        let generation = self.next_generation.get().wrapping_add(1);
        self.next_generation.set(generation);
        self.commit_timing
            .borrow_mut()
            .insert(key.clone(), (generation, wake_at));
        let scheduled = Rc::clone(&self.commit_timing);
        let timer_key = key.clone();
        let output = output.clone();
        if let Err(err) = loop_handle.insert_source(
            smithay::reexports::calloop::timer::Timer::from_duration(delay),
            move |_, _, state| {
                let is_current = scheduled
                    .borrow()
                    .get(&timer_key)
                    .is_some_and(|(current, _)| *current == generation);
                if is_current {
                    scheduled.borrow_mut().remove(&timer_key);
                    let clock = Clock::<Monotonic>::new();
                    let target = clock.now() + output_frame_callback_delay(&output);
                    crate::backend::wayland::render::frame::service_commit_timing(
                        state, &output, target,
                    );
                    state.request_output_render(&output);
                }
                smithay::reexports::calloop::timer::TimeoutAction::Drop
            },
        ) {
            self.commit_timing.borrow_mut().remove(&key);
            log::warn!("failed to arm commit-timing wakeup for {key:?}: {err}");
        }
    }

    fn next_presentation_delay(&self, key: &K, period: Duration, now: Instant) -> Duration {
        self.presentation_phase
            .borrow()
            .get(key)
            .map_or(period, |last| next_phase_delay(*last, now, period))
    }
}

fn output_frame_callback_delay(output: &Output) -> Duration {
    output
        .current_mode()
        .and_then(|mode| {
            let refresh = u64::try_from(mode.refresh).ok()?;
            (refresh > 0).then(|| Duration::from_nanos(1_000_000_000_000u64 / refresh))
        })
        .unwrap_or_else(|| Duration::from_millis(16))
}

fn commit_timing_wake_delay(
    deadline: Duration,
    now: Duration,
    refresh_period: Duration,
) -> Duration {
    deadline.saturating_sub(now).saturating_sub(refresh_period)
}

fn next_phase_delay(last: Instant, now: Instant, period: Duration) -> Duration {
    let elapsed = now.saturating_duration_since(last);
    let period_nanos = period.as_nanos();
    if period_nanos == 0 {
        return Duration::ZERO;
    }
    let remainder = Duration::from_nanos(
        u64::try_from(elapsed.as_nanos() % period_nanos)
            .expect("a refresh-phase remainder is smaller than one Duration"),
    );
    if remainder.is_zero() {
        period
    } else {
        period - remainder
    }
}

#[cfg(test)]
mod presentation_scheduler_tests {
    use super::{commit_timing_wake_delay, next_phase_delay};
    use std::time::{Duration, Instant};

    #[test]
    fn timed_commit_wakes_one_refresh_before_deadline() {
        assert_eq!(
            commit_timing_wake_delay(
                Duration::from_millis(100),
                Duration::from_millis(20),
                Duration::from_millis(16),
            ),
            Duration::from_millis(64)
        );
    }

    #[test]
    fn imminent_and_stale_deadlines_wake_immediately() {
        for deadline in [10, 20, 25] {
            assert_eq!(
                commit_timing_wake_delay(
                    Duration::from_millis(deadline),
                    Duration::from_millis(20),
                    Duration::from_millis(16),
                ),
                Duration::ZERO
            );
        }
    }

    #[test]
    fn callback_delay_stays_aligned_to_observed_presentation_phase() {
        let phase = Instant::now();
        let period = Duration::from_millis(10);
        assert_eq!(
            next_phase_delay(phase, phase + Duration::from_millis(24), period),
            Duration::from_millis(6)
        );
        assert_eq!(next_phase_delay(phase, phase, period), period);
    }
}
/// Run the shared Wayland tick and convert model changes into one compositor
/// redraw request. DRM and winit then consume that request using their own
/// output submission machinery.
pub(crate) fn event_loop_tick_and_request_render(
    wm: &mut Wm,
    state: &mut WaylandState,
    ipc_server: &mut Option<crate::ipc::IpcServer>,
) {
    super::dispatch::drain_command_queue(wm, state);
    crate::backend::wayland::compositor::protocols::ext_workspace::refresh(state);
    let tick = crate::runtime::event_loop_tick_with_options(
        wm,
        ipc_server,
        crate::runtime::TickOptions {
            defer_layout_while_animations_active: true,
            animations_active: state.has_active_window_animations(),
        },
    );
    // Moving surfaces under a stationary pointer must update Wayland pointer
    // protocol focus in every mode. The synthetic source is kept distinct so
    // only `force` may turn that protocol refresh into keyboard focus.
    if tick.layout_applied
        && let (Some(pointer), Some(keyboard)) =
            (state.seat.get_pointer(), state.seat.get_keyboard())
    {
        crate::backend::wayland::input::pointer::motion::process_pointer_motion_command(
            wm,
            state,
            &pointer,
            &keyboard,
            crate::backend::wayland::commands::PointerMotionCommand::Refresh { time_msec: 0 },
        );
    }
    // Commit external protocol projections only after every shared and
    // Wayland-specific operation belonging to this tick has completed.
    let selection_transition = wm.focus.take_pending_selection();
    state.reconcile_foreign_toplevel_selection(selection_transition);
    dismiss_invalid_native_systray_menu(wm, state);
    if tick.ipc_handled
        || tick.monitor_config_applied
        || tick.layout_applied
        || tick.systray_updated
    {
        state.request_render();
    }
}

fn dismiss_invalid_native_systray_menu(wm: &Wm, state: &mut WaylandState) {
    let Some(active) = state.active_systray_menu().cloned() else {
        return;
    };
    let opening_view_is_current = wm
        .core
        .model
        .monitor(active.monitor_id)
        .is_some_and(|monitor| monitor.selected_tags() == active.opened_tags);
    let item_still_exists = wm
        .bar
        .systray_host
        .tray
        .items
        .iter()
        .any(|item| item.service == active.service && item.path == active.path);
    if !wm.core.config.systray.show || !opening_view_is_current || !item_still_exists {
        state.dismiss_native_systray_menu();
    }
}

/// Run compositor-space sync and animation progression in one place, then
/// preserve the resulting redraw in the shared Wayland scheduler.
pub(crate) fn process_animations_and_request_render(state: &mut WaylandState) {
    let space_synced = if state.take_space_sync_pending() {
        state.sync_space_from_globals();
        // Output membership for foreign-toplevel clients must be computed
        // from post-arrange geometry: this is the point in the tick where
        // pending layouts have been applied and the space reconciled.
        state.refresh_all_foreign_toplevels();
        true
    } else {
        false
    };
    if state.shortcut_recovery_needs_tick() {
        state.tick_shortcut_recovery(Instant::now());
    }
    if state.has_active_animations() {
        state.tick_animations();
        // A retarget that just settled moves windows between outputs after
        // the refresh above already ran; catch up once animations drain.
        if !state.has_active_animations() {
            state.refresh_all_foreign_toplevels();
        }
    }

    // Animation ticks enqueue output-local redraws themselves. Space sync can
    // affect arbitrary windows, so it remains conservatively global.
    if space_synced {
        state.request_render();
    }
}
