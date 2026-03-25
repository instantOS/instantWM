use smithay::desktop::Window;
use smithay::utils::{Logical, Point};
use smithay::wayland::shell::xdg::ToplevelSurface;

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::compositor::state::WindowIdMarker;
use crate::types::WindowId;

impl WaylandState {
    /// Map a new toplevel surface (from XDG shell).
    pub fn map_new_toplevel(&mut self, surface: ToplevelSurface) -> WindowId {
        let window = Window::new_wayland_window(surface);
        let window_id = self.alloc_window_id();
        let _ = window
            .user_data()
            .get_or_insert_threadsafe(|| WindowIdMarker {
                id: window_id,
                is_overlay: false,
            });

        self.space.map_element(window.clone(), (0, 0), false);
        self.window_index.insert(window_id, window.clone());
        self.ensure_client_for_window(window_id);

        if let Some(title) = self.window_title(window_id)
            && let Some(g) = self.globals_mut()
            && let Some(client) = g.clients.get_mut(&window_id)
        {
            client.name = title;
        }

        if window.toplevel().is_some() {
            let (w, h) = self
                .globals()
                .and_then(|g| g.clients.get(&window_id).map(|c| (c.geo.w, c.geo.h)))
                .unwrap_or((Self::MIN_WL_DIM, Self::MIN_WL_DIM));
            let target = (w.max(Self::MIN_WL_DIM), h.max(Self::MIN_WL_DIM));
            let size =
                smithay::utils::Size::<i32, smithay::utils::Logical>::new(target.0, target.1);
            self.send_toplevel_configure(&window, Some(size));
            self.last_configured_size.insert(window_id, target);
        }
        if let Some(g) = self.globals_mut() {
            // Set mon.sel for the newly created window so the WM layer knows
            // to focus it when it runs the next arrange/focus pass.
            if let Some(monitor_id) = g.clients.monitor_id(window_id)
                && let Some(mon) = g.monitor_mut(monitor_id)
            {
                mon.sel = Some(window_id);
            }
            g.dirty.layout = true;
            g.dirty.space = true;
        }
        // Apply seat focus immediately so the new window can receive input.
        self.set_focus(window_id);
        self.create_foreign_toplevel(window_id);
        window_id
    }

    /// Map a window (make it visible).
    pub fn map_window(&mut self, window: WindowId) {
        // Get the location from the space if the element is already mapped,
        // otherwise use the client's stored geometry to avoid animating from (0,0)
        let is_already_mapped = self
            .find_window(window)
            .is_some_and(|w| self.space.elements().any(|e| e == w));

        // If the window is already mapped, calling `map_element` will unnecessarily
        // pull it to the top of the stack and disrupt the Z-order.
        if is_already_mapped {
            return;
        }

        if let Some(element) = self.window_index.get(&window).cloned() {
            let is_mapped = self.space.elements().any(|w| w == &element);
            if !is_mapped {
                let loc: Point<i32, Logical> = self
                    .globals()
                    .and_then(|g| g.clients.get(&window))
                    .map(|c| Point::from((c.geo.x + c.border_width, c.geo.y + c.border_width)))
                    .unwrap_or(Point::from((0, 0)));
                self.window_animations.remove(&window);
                self.space.map_element(element.clone(), loc, false);

                // If this window was the pending focus target (set by focus_soft
                // before arrange/show_hide ran), re-apply keyboard focus now that
                // the window is actually in the space and reachable by set_focus.
                if self.focused_window() == Some(window) {
                    self.set_focus(window);
                }
            }
        }
    }

    /// Unmap a window (hide it).
    ///
    /// Clears Smithay seat focus if this window holds it, but does **not**
    /// touch `mon.sel`. The WM layer will reconcile focus after the
    /// show/hide pass.
    pub fn unmap_window(&mut self, window: WindowId) {
        if let Some(element) = self.window_index.get(&window).cloned() {
            self.space.unmap_elem(&element);
        }
        self.window_animations.remove(&window);
        self.last_configured_size.remove(&window);
        self.clear_seat_focus_if_focused(window);
        if let Some(g) = self.globals_mut() {
            g.dirty.layout = true;
            g.dirty.space = true;
        }
    }

    /// Remove all tracking for a window.
    ///
    /// Clears seat focus if this window holds it, but does **not** touch
    /// `mon.sel`. The caller is responsible for WM focus reconciliation.
    pub(crate) fn remove_window_tracking(&mut self, window: WindowId) {
        if let Some(element) = self.window_index.get(&window).cloned() {
            self.space.unmap_elem(&element);
        }
        self.window_index.remove(&window);
        self.window_animations.remove(&window);
        self.last_configured_size.remove(&window);
        self.clear_seat_focus_if_focused(window);
        self.close_foreign_toplevel(window);
        if let Some(g) = self.globals_mut() {
            g.dirty.layout = true;
            g.dirty.space = true;
        }
    }

    /// Close a window.
    pub fn close_window(&mut self, window: WindowId) -> bool {
        let Some(element) = self.find_window(window).cloned() else {
            return false;
        };
        if let Some(x11) = element.x11_surface() {
            let _ = x11.close();
            return true;
        }
        if let Some(toplevel) = element.toplevel() {
            toplevel.send_close();
            return true;
        }
        false
    }
}
