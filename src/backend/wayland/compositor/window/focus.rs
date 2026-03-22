use smithay::utils::IsAlive;
use smithay::utils::SERIAL_COUNTER;

use crate::backend::wayland::compositor::focus::KeyboardFocusTarget;
use crate::backend::wayland::compositor::state::WindowIdMarker;
use crate::backend::wayland::compositor::{WaylandRuntime, WaylandState};
use crate::types::WindowId;

impl WaylandState {
    /// Apply keyboard focus to a window on the Smithay seat.
    ///
    /// This is a **seat-only** operation. It:
    /// 1. Deactivates the previously focused window (via Smithay activated state)
    /// 2. Activates the new window
    /// 3. Sets Smithay keyboard focus
    ///
    /// It does **not** update `mon.sel`. The WM layer (`focus_generic` /
    /// `focus_soft`) is the single authority for `mon.sel`.
    pub fn set_focus(&mut self, window: WindowId) {
        let serial = SERIAL_COUNTER.next_serial();
        let focus_window = self.find_window(window).cloned();

        // If the window doesn't exist in our index, clear seat focus.
        if focus_window.is_none() && !self.window_index.contains_key(&window) {
            log::warn!(
                "set_focus: window {:?} not found, clearing seat focus",
                window
            );
            self.clear_seat_focus();
            return;
        }

        // Check if window is alive - don't focus dying windows
        if let Some(ref win) = focus_window
            && !win.alive()
        {
            log::debug!(
                "set_focus: window {:?} is dying, clearing seat focus",
                window
            );
            self.clear_seat_focus();
            return;
        }

        let focus = focus_window.clone().map(KeyboardFocusTarget::Window);

        // Get the previously focused window from WM state (mon.sel)
        let previously_focused = self.wm.g.selected_win().filter(|&old_id| old_id != window);

        // Deactivate the previously focused window
        if let Some(old_id) = previously_focused
            && let Some(old_window) = self.window_index.get(&old_id).cloned()
            && old_window.set_activated(false)
        {
            self.send_toplevel_configure(&old_window, None);
        }

        // Activate the new window and set keyboard focus
        if let Some(new_window) = focus_window {
            if new_window.set_activated(true) {
                self.send_toplevel_configure(&new_window, None);
            }
            // Set keyboard focus on the Smithay seat
            if let Some(keyboard) = self.seat.get_keyboard() {
                keyboard.set_focus(WaylandRuntime::from_state_mut(self), focus, serial);
            } else {
                log::warn!(
                    "set_focus: no keyboard seat available for window {:?}",
                    window
                );
            }
        }
    }

    /// This returns the window that the WM thinks should be focused.
    /// For the actual Smithay seat focus, use `seat.get_keyboard().current_focus()`.
    pub fn focused_window(&self) -> Option<WindowId> {
        self.wm.g.selected_win()
    }

    /// Check whether the Smithay keyboard seat is currently focused on the
    /// X11 surface with the given `window_id`.
    pub(crate) fn is_x11_surface_focused(&self, window_id: u32) -> bool {
        self.seat
            .get_keyboard()
            .and_then(|k| k.current_focus())
            .is_some_and(|focus| {
                if let KeyboardFocusTarget::Window(w) = focus {
                    w.x11_surface()
                        .is_some_and(|x11| x11.window_id() == window_id)
                } else {
                    false
                }
            })
    }

    /// Clear keyboard focus on the Smithay seat only.
    ///
    /// This does **not** touch `mon.sel`. Use this when the seat focus
    /// needs to be cleared (e.g. the focused surface is dying) but the WM
    /// layer will reconcile `mon.sel` separately.
    pub(crate) fn clear_seat_focus(&mut self) {
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(
                WaylandRuntime::from_state_mut(self),
                None::<KeyboardFocusTarget>,
                serial,
            );
        }
    }

    /// Clear seat focus if the given window currently holds it.
    ///
    /// Checks the actual Smithay seat keyboard focus (not `mon.sel`).
    /// Used when a window is unmapped or removed to avoid leaving the
    /// keyboard seat pointing at a dead surface.
    pub(crate) fn clear_seat_focus_if_focused(&mut self, window: WindowId) {
        let is_seat_focused = self.is_seat_focused_on(window);
        if is_seat_focused {
            self.clear_seat_focus();
        }
    }

    /// Check if the Smithay seat keyboard focus is currently on the given window.
    pub(crate) fn is_seat_focused_on(&self, window: WindowId) -> bool {
        self.seat
            .get_keyboard()
            .and_then(|k| k.current_focus())
            .is_some_and(|focus| match focus {
                KeyboardFocusTarget::Window(w) => w
                    .user_data()
                    .get::<WindowIdMarker>()
                    .is_some_and(|m| m.id == window),
                _ => false,
            })
    }

    /// Restore seat focus after an overlay (e.g., dmenu) is closed, or
    /// after a window was destroyed and `mon.sel` was cleared.
    ///
    /// If `mon.sel` is valid and alive, applies seat focus to it.
    /// If `mon.sel` is `None` or stale, walks the monitor stack to find
    /// the next visible window and updates `mon.sel` before focusing.
    pub(crate) fn restore_focus_after_overlay(&mut self) {
        // First, try mon.sel as-is.
        let valid_sel = self.wm.g.selected_win().filter(|&w| {
            self.window_index.contains_key(&w)
                && self.window_index.get(&w).is_some_and(|win| win.alive())
        });

        if let Some(win) = valid_sel {
            // mon.sel is valid — just apply seat focus.
            self.set_focus(win);
            return;
        }

        // mon.sel is None or stale.  Walk the stack to find the next
        // visible window and update mon.sel.
        let sel_mon_id = self.wm.g.selected_monitor_id();
        let next = self
            .wm
            .g
            .monitor(sel_mon_id)
            .and_then(|m| m.first_visible_client(self.wm.g.clients.map()));
        let recovered = if let Some(mon) = self.wm.g.monitor_mut(sel_mon_id) {
            mon.sel = next;
            next
        } else {
            next
        };

        if let Some(win) = recovered {
            self.set_focus(win);
        } else {
            self.clear_seat_focus();
        }
    }
}
