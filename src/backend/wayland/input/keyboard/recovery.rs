//! Emergency recovery from an application-owned keyboard grab.
//!
//! This is deliberately not part of the ordinary keybinding system.  It is a
//! compositor safety invariant with fixed semantics, available only while an
//! application is suppressing compositor shortcuts.

use std::time::{Duration, Instant};

use smithay::input::keyboard::Keycode;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::IsAlive;
use smithay::wayland::input_method::InputMethodSeat;
use smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat;
use smithay::wayland::seat::WaylandFocus;

use crate::backend::wayland::compositor::WaylandState;

pub(crate) const SHORTCUT_RECOVERY_HOLD: Duration = Duration::from_secs(2);
pub(crate) const SHORTCUT_RECOVERY_INDICATOR_DELAY: Duration = Duration::from_millis(200);
const SHORTCUT_RECOVERY_CONFIRMATION: Duration = Duration::from_millis(160);

#[derive(Debug)]
struct ArmedShortcutRecovery {
    keycode: Keycode,
    surface: WlSurface,
    output_name: Option<String>,
    started_at: Instant,
}

#[derive(Debug)]
struct ShortcutRecoveryConfirmation {
    output_name: Option<String>,
    started_at: Instant,
}

/// Runtime state for the compositor's fixed grab-recovery gesture.
#[derive(Debug, Default)]
pub(crate) struct ShortcutRecoveryState {
    armed: Option<ArmedShortcutRecovery>,
    /// Keep consuming repeats and the eventual release after the deadline.
    consumed_keycode: Option<Keycode>,
    /// Keep a full-width confirmation frame visible briefly after success.
    confirmation: Option<ShortcutRecoveryConfirmation>,
    /// A client which the user explicitly escaped may not immediately install
    /// another inhibitor/grab.  Leaving its surface clears this denial.
    bypassed_surface: Option<WlSurface>,
}

impl ShortcutRecoveryState {
    fn is_armed(&self) -> bool {
        self.armed.is_some()
    }

    fn needs_tick(&self) -> bool {
        self.armed.is_some() || self.confirmation.is_some()
    }

    fn owns_key(&self, keycode: Keycode) -> bool {
        self.consumed_keycode == Some(keycode)
    }

    fn target_matches(&self, surface: &WlSurface) -> bool {
        self.bypassed_surface.as_ref() == Some(surface)
    }

    fn progress_for_output(&self, output_name: &str, now: Instant) -> Option<f64> {
        if let Some(confirmation) = self.confirmation.as_ref()
            && confirmation
                .output_name
                .as_deref()
                .is_none_or(|name| name == output_name)
            && now.saturating_duration_since(confirmation.started_at)
                < SHORTCUT_RECOVERY_CONFIRMATION
        {
            return Some(1.0);
        }
        let armed = self.armed.as_ref()?;
        if armed
            .output_name
            .as_deref()
            .is_some_and(|name| name != output_name)
        {
            return None;
        }
        normalized_progress(armed.started_at, now)
    }
}

fn normalized_progress(started_at: Instant, now: Instant) -> Option<f64> {
    let elapsed = now.saturating_duration_since(started_at);
    (elapsed >= SHORTCUT_RECOVERY_INDICATOR_DELAY)
        .then(|| (elapsed.as_secs_f64() / SHORTCUT_RECOVERY_HOLD.as_secs_f64()).clamp(0.0, 1.0))
}

impl WaylandState {
    /// Arm (or continue holding) the fixed recovery gesture for `surface`.
    pub(crate) fn arm_shortcut_recovery(&mut self, keycode: Keycode, surface: WlSurface) {
        if self
            .runtime
            .shortcut_recovery
            .armed
            .as_ref()
            .is_some_and(|armed| armed.keycode == keycode && armed.surface == surface)
        {
            return;
        }

        let output_name = self
            .shortcut_recovery_output(&surface)
            .map(|output| output.name());
        self.runtime.shortcut_recovery.armed = Some(ArmedShortcutRecovery {
            keycode,
            surface,
            output_name,
            started_at: Instant::now(),
        });
        self.runtime.shortcut_recovery.consumed_keycode = Some(keycode);
        self.runtime.shortcut_recovery.confirmation = None;
        self.request_render();
    }

    /// Cancel an incomplete gesture.  Its trigger-key release is still
    /// intercepted by the normal paired-release bookkeeping.
    pub(crate) fn cancel_shortcut_recovery(&mut self) {
        if self.runtime.shortcut_recovery.armed.take().is_some() {
            self.request_render();
        }
    }

    pub(crate) fn shortcut_recovery_owns_key(&self, keycode: Keycode) -> bool {
        self.runtime.shortcut_recovery.owns_key(keycode)
    }

    /// Finish ownership of the trigger key on its physical release.
    pub(crate) fn release_shortcut_recovery_key(&mut self, keycode: Keycode) -> bool {
        if !self.shortcut_recovery_owns_key(keycode) {
            return false;
        }
        self.runtime.shortcut_recovery.consumed_keycode = None;
        self.cancel_shortcut_recovery();
        true
    }

    pub(crate) fn shortcut_recovery_is_armed(&self) -> bool {
        self.runtime.shortcut_recovery.is_armed()
    }

    pub(crate) fn shortcut_recovery_needs_tick(&self) -> bool {
        self.runtime.shortcut_recovery.needs_tick()
    }

    pub(crate) fn shortcut_recovery_bypasses(&self, surface: &WlSurface) -> bool {
        self.runtime.shortcut_recovery.target_matches(surface)
    }

    /// Update recovery ownership after a seat focus change.
    pub(crate) fn shortcut_recovery_focus_changed(&mut self, focused: Option<&WlSurface>) {
        let armed_matches = self
            .runtime
            .shortcut_recovery
            .armed
            .as_ref()
            .is_some_and(|armed| focused == Some(&armed.surface));
        if !armed_matches {
            self.cancel_shortcut_recovery();
        }

        let bypass_matches = self
            .runtime
            .shortcut_recovery
            .bypassed_surface
            .as_ref()
            .is_some_and(|surface| focused == Some(surface));
        if !bypass_matches {
            self.runtime.shortcut_recovery.bypassed_surface = None;
        }

        // An inhibitor explicitly deactivated by recovery becomes eligible
        // again after the user leaves it and later returns focus to it.
        if let Some(surface) = focused
            && !self.shortcut_recovery_bypasses(surface)
            && let Some(inhibitor) = self.seat.keyboard_shortcuts_inhibitor_for_surface(surface)
            && !inhibitor.is_active()
        {
            inhibitor.activate();
        }
    }

    /// Advance the safety gesture and break the grab at its deadline.
    pub(crate) fn tick_shortcut_recovery(&mut self, now: Instant) {
        if self
            .runtime
            .shortcut_recovery
            .confirmation
            .as_ref()
            .is_some_and(|confirmation| {
                now.saturating_duration_since(confirmation.started_at)
                    >= SHORTCUT_RECOVERY_CONFIRMATION
            })
        {
            self.runtime.shortcut_recovery.confirmation = None;
            self.request_render();
        } else if self.runtime.shortcut_recovery.confirmation.is_some() {
            self.request_render();
        }

        let Some(armed) = self.runtime.shortcut_recovery.armed.as_ref() else {
            return;
        };

        let focused_surface = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .and_then(|focus| WaylandFocus::wl_surface(&focus).map(|surface| surface.into_owned()));
        let still_focused = focused_surface.as_ref() == Some(&armed.surface);
        let target_alive = armed.surface.alive();
        let suppression_still_active = self
            .seat
            .keyboard_shortcuts_inhibitor_for_surface(&armed.surface)
            .is_some_and(|inhibitor| inhibitor.is_active())
            || self.seat.get_keyboard().is_some_and(|keyboard| {
                keyboard.is_grabbed() && !self.seat.input_method().keyboard_grabbed()
            });

        if self.is_locked() || !target_alive || !still_focused || !suppression_still_active {
            self.cancel_shortcut_recovery();
            return;
        }

        if now.saturating_duration_since(armed.started_at) < SHORTCUT_RECOVERY_HOLD {
            if let Some(name) = armed.output_name.clone() {
                self.request_output_name_render(name);
            } else {
                self.request_render();
            }
            return;
        }

        let surface = armed.surface.clone();
        let output_name = armed.output_name.clone();
        self.runtime.shortcut_recovery.armed = None;
        self.runtime.shortcut_recovery.bypassed_surface = Some(surface.clone());
        self.runtime.shortcut_recovery.confirmation = Some(ShortcutRecoveryConfirmation {
            output_name,
            started_at: now,
        });

        if let Some(inhibitor) = self.seat.keyboard_shortcuts_inhibitor_for_surface(&surface)
            && inhibitor.is_active()
        {
            inhibitor.inactivate();
        }
        if let Some(keyboard) = self.seat.get_keyboard()
            && keyboard.is_grabbed()
            && !self.seat.input_method().keyboard_grabbed()
        {
            keyboard.unset_grab(self);
        }

        log::info!("restored compositor shortcuts after emergency hold");
        self.request_render();
    }

    pub(crate) fn shortcut_recovery_progress(&self, output: &Output) -> Option<f64> {
        self.runtime
            .shortcut_recovery
            .progress_for_output(&output.name(), Instant::now())
    }

    fn shortcut_recovery_output(&self, surface: &WlSurface) -> Option<Output> {
        if let Some(window_id) = self.window_id_for_surface(surface)
            && let Some(window) = self.find_window(window_id)
        {
            let outputs = self.outputs_for_window_geometry(window);
            if let Some(pointer_output) = self.output_at_point(self.runtime.pointer_location)
                && outputs.contains(&pointer_output)
            {
                return Some(pointer_output);
            }
            if let Some(output) = outputs.into_iter().next() {
                return Some(output);
            }
        }

        self.output_at_point(self.runtime.pointer_location)
            .or_else(|| self.space.outputs().next().cloned())
    }

    fn output_at_point(
        &self,
        point: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> Option<Output> {
        self.space.outputs().find_map(|output| {
            self.space
                .output_geometry(output)
                .is_some_and(|geometry| geometry.to_f64().contains(point))
                .then(|| output.clone())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_hidden_briefly_then_clamped() {
        let now = Instant::now();
        assert_eq!(normalized_progress(now, now), None);
        assert_eq!(
            normalized_progress(now, now + Duration::from_secs(1)),
            Some(0.5)
        );
        assert_eq!(
            normalized_progress(now, now + Duration::from_secs(3)),
            Some(1.0)
        );
    }
}
