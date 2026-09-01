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

/// Coalesces callback-only surface commits and delivers them at output refresh
/// cadence without forcing either rendering backend to submit an empty frame.
#[derive(Debug)]
pub(crate) struct FrameCallbackTimerGuard<K> {
    armed: Rc<RefCell<HashMap<K, u64>>>,
    next_generation: Cell<u64>,
}

impl<K> Default for FrameCallbackTimerGuard<K> {
    fn default() -> Self {
        Self {
            armed: Rc::new(RefCell::new(HashMap::new())),
            next_generation: Cell::new(0),
        }
    }
}

impl<K> FrameCallbackTimerGuard<K>
where
    K: Clone + Debug + Eq + Hash + 'static,
{
    pub(crate) fn arm(
        &self,
        key: K,
        loop_handle: &LoopHandle<'_, WaylandState>,
        output: &Output,
        start_time: Instant,
    ) {
        if self.armed.borrow().contains_key(&key) {
            return;
        }

        let generation = self.next_generation.get().wrapping_add(1);
        self.next_generation.set(generation);
        self.armed.borrow_mut().insert(key.clone(), generation);

        let output = output.clone();
        let delay = output_frame_callback_delay(&output);
        let armed_for_timer = Rc::clone(&self.armed);
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
                .armed
                .borrow()
                .get(&key)
                .is_some_and(|current| *current == generation);
            if is_current {
                self.armed.borrow_mut().remove(&key);
            }
            log::warn!("failed to arm frame-callback timer for {key:?}: {err}");
        }
    }

    pub(crate) fn disarm(&self, key: &K) {
        self.armed.borrow_mut().remove(key);
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
