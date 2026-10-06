//! Owned backend-neutral policy state and pending runtime work.
use super::{CoreState, PendingWork};
use crate::bar::BarState;
use crate::client::focus::FocusState;
use crate::model::WmModel;
use crate::types::{MonitorId, Rect, WindowId};

/// The shared policy owner. Contexts borrow this aggregate directly instead
/// of storing references to each field alongside an independently borrowed
/// compositor. This lets native callbacks borrow their complete handler state
/// after a model operation ends, with exclusivity enforced by Rust.
///
/// Data follows one path: `core.state` owns model/config/interaction and
/// `core.work` owns pending work. Pass those components to leaf operations,
/// rather than adding forwarding getters that hide their ownership paths.
pub struct WmCore {
    pub(crate) state: CoreState,
    pub(crate) work: PendingWork,
    pub(crate) running: bool,
    pub bar: BarState,
    pub focus: FocusState,
}
impl Default for WmCore {
    fn default() -> Self {
        Self {
            state: CoreState::default(),
            work: PendingWork::default(),
            running: true,
            bar: BarState::default(),
            focus: FocusState::default(),
        }
    }
}
impl WmCore {
    /// Drain StatusNotifier worker events. Returns `true` when tray content
    /// changed and the bar must be redrawn.
    pub fn poll_systray(&mut self) -> bool {
        self.configure_tray_icons();
        let changed = self.bar.systray_host.poll();
        if changed {
            self.bar.mark_dirty();
        }
        changed
    }

    pub(crate) fn configure_tray_icons(&mut self) {
        let config = &self.state.config.systray;
        // Monitor bar heights already include output scaling. One source at
        // the largest required resolution can serve every bar without upscaling.
        let height = self
            .state
            .model
            .monitors_iter_all()
            .map(|monitor| {
                let padding = crate::systray::visual_padding(monitor.bar_height, config.spacing);
                (monitor.bar_height - 2 * padding).max(1) as u32
            })
            .max()
            .unwrap_or(24);
        if let Some(runtime) = self.bar.systray_host.runtime.as_mut() {
            runtime.configure_icons(crate::systray::status_notifier::IconSettings {
                theme: config.icon_theme.clone(),
                height,
            });
        }
    }

    pub fn start_status_sources(&mut self) {
        self.bar
            .status_sources
            .start(self.state.config.status_command.as_deref());
    }

    /// Return a managed client's current logical geometry.
    #[inline]
    pub fn client_geo(&self, win: WindowId) -> Option<Rect> {
        self.state.model.client(win).map(|client| client.geo)
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn queue_layout_for_all_monitors(&mut self) {
        self.work.layout.mark_all();
    }

    pub fn queue_layout_for_all_monitors_urgent(&mut self) {
        self.work.layout.mark_all_urgent();
    }

    pub fn queue_layout_for_monitor(&mut self, monitor_id: MonitorId) {
        self.work.layout.mark_monitor(monitor_id);
    }

    pub fn queue_layout_for_monitor_urgent(&mut self, monitor_id: MonitorId) {
        self.work.layout.mark_monitor_urgent(monitor_id);
    }

    pub fn queue_layout_for_client(&mut self, win: WindowId) {
        if let Some(monitor_id) = self.state.model.monitor_of_client(win) {
            self.work.layout.mark_monitor(monitor_id);
        }
    }

    /// Queue the first authoritative layout for a newly managed window and
    /// its post-layout entrance transition as one lifecycle operation.
    ///
    /// First layout is urgent because the Wayland surface remains intentionally
    /// unmapped until this work has assigned usable geometry.
    pub fn queue_initial_window_layout(&mut self, win: WindowId, monitor_id: MonitorId) {
        self.work.layout.mark_monitor_urgent(monitor_id);
        self.work.spawn_animations.insert(win);
    }

    pub fn queue_monitor_config_apply(&mut self) {
        self.work.queue_monitor_config_apply();
    }

    pub fn queue_input_config_apply(&mut self) {
        self.work.queue_input_config_apply();
    }

    pub fn queue_cursor_config_apply(&mut self) {
        self.work.queue_cursor_config_apply();
    }

    /// Run a model transaction and record any resulting global-selection
    /// transition. All production mutations that can affect selection cross
    /// this boundary, including indirect removal/reassignment effects.
    pub fn mutate_selection<R>(&mut self, mutation: impl FnOnce(&mut WmModel) -> R) -> R {
        self.mutate_state_selection(|state| mutation(&mut state.model))
    }

    pub fn mutate_state_selection<R>(&mut self, mutation: impl FnOnce(&mut CoreState) -> R) -> R {
        let previous = self.state.model.selected_win();
        let result = mutation(&mut self.state);
        let current = self.state.model.selected_win();
        self.focus.record_selection(previous, current);
        result
    }

    pub fn select_monitor(&mut self, monitor_id: MonitorId) -> bool {
        self.mutate_selection(|model| {
            if !model.can_change_selected_monitor(monitor_id) {
                return false;
            }
            model.set_selected_monitor(monitor_id);
            true
        })
    }

    pub fn select_on_monitor(&mut self, monitor_id: MonitorId, selected: Option<WindowId>) -> bool {
        self.mutate_selection(|model| {
            let Some(monitor) = model.monitor_mut(monitor_id) else {
                return false;
            };
            if monitor.selected == selected {
                return false;
            }
            monitor.set_selected(selected);
            true
        })
    }
}
