use log::debug;
use smithay::desktop::Window;
use smithay::utils::{Logical, Point};
use smithay::wayland::shell::xdg::ToplevelSurface;

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::compositor::state::WindowIdMarker;
use crate::types::WindowId;

impl WaylandState {
    pub(crate) fn setup_managed_window(&mut self, surface: ToplevelSurface) -> WindowId {
        let window_id = self.register_toplevel(surface, false);
        self.create_foreign_toplevel(window_id);
        window_id
    }

    pub(crate) fn setup_native_systray_menu(
        &mut self,
        surface: ToplevelSurface,
        request: crate::systray::status_notifier::NativeMenuRequest,
    ) -> Result<WindowId, ToplevelSurface> {
        let Some((monitor_id, opened_tags, work_rect)) = self.globals().and_then(|globals| {
            let monitor_id = globals
                .model
                .monitors
                .find_monitor_at_pointer(request.anchor)
                .or_else(|| {
                    globals
                        .model
                        .selected_monitor_opt()
                        .map(|monitor| monitor.id())
                })?;
            let monitor = globals.monitor(monitor_id)?;
            Some((monitor_id, monitor.selected_tags(), monitor.work_rect))
        }) else {
            return Err(surface);
        };

        let window_id = self.register_toplevel(surface, true);
        let window = self
            .find_window(window_id)
            .expect("a newly registered toplevel must be indexed")
            .clone();
        let geometry = window.geometry();
        let requested =
            crate::types::Rect::new(0, 0, geometry.size.w.max(1), geometry.size.h.max(1));
        let rect = crate::systray::native_menu_rect(work_rect, requested, request.anchor);
        let location = (
            rect.x.saturating_sub(geometry.loc.x),
            rect.y.saturating_sub(geometry.loc.y),
        );
        self.space.map_element(window.clone(), location, false);
        self.space.raise_element(&window, true);

        if let Some(previous) = self.runtime.active_systray_menu.replace(
            crate::systray::status_notifier::ActiveNativeMenu {
                win: window_id,
                service: request.service,
                path: request.path,
                monitor_id,
                opened_tags,
                close_requested: false,
            },
        ) && !previous.close_requested
        {
            self.close_window(previous.win);
        }

        self.set_focus(window_id);
        self.request_visible_window_render(&window);
        self.request_render();
        Ok(window_id)
    }

    /// Register a toplevel in compositor space without assigning desktop policy.
    fn register_toplevel(&mut self, surface: ToplevelSurface, is_overlay: bool) -> WindowId {
        let window = Window::new_wayland_window(surface);
        let window_id = self.alloc_window_id();
        let _ = window
            .user_data()
            .get_or_insert_threadsafe(|| WindowIdMarker {
                id: window_id,
                is_overlay,
            });

        // Map at (0,0) initially. The WM will move it during the layout pass.
        self.space.map_element(window.clone(), (0, 0), false);
        self.window_index.insert(window_id, window.clone());

        // Refresh the Window's internal geometry cache.
        window.on_commit();

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
            debug!("map_window({window:?}): no-op, already mapped");
            return;
        }

        if let Some(element) = self.window_index.get(&window).cloned() {
            let is_mapped = self.space.elements().any(|w| w == &element);
            if !is_mapped {
                let Some(loc): Option<Point<i32, Logical>> = self
                    .globals()
                    .and_then(|g| g.model.client(window))
                    .map(|c| Point::from((c.geo.x + c.border_width, c.geo.y + c.border_width)))
                else {
                    return;
                };
                self.drop_window_animation(window);
                self.space.map_element(element.clone(), loc, false);
                self.request_visible_window_render(&element);

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
        let Some(element) = self.window_index.get(&window).cloned() else {
            debug!("unmap_window({window:?}): no-op, window not found");
            return;
        };
        let is_mapped = self.space.elements().any(|w| w == &element);
        if !is_mapped {
            debug!("unmap_window({window:?}): no-op, already unmapped");
            return;
        }

        // Invalidate its old outputs before removing the geometry from Space.
        self.request_visible_window_render(&element);
        self.space.unmap_elem(&element);
        self.drop_window_animation(window);
        self.last_configured_size.remove(&window);
        self.clear_seat_focus_if_focused(window);
        self.request_space_sync();
    }

    /// Remove all tracking for a window.
    ///
    /// Clears seat focus if this window holds it, but does **not** touch
    /// `mon.sel`. The caller is responsible for WM focus reconciliation.
    pub(crate) fn remove_window_tracking(&mut self, window: WindowId) {
        self.clear_active_systray_menu(window);
        if let Some(element) = self.window_index.get(&window).cloned()
            && self.space.elements().any(|mapped| mapped == &element)
        {
            // Invalidate its old outputs before removing the geometry from Space.
            self.request_visible_window_render(&element);
            self.space.unmap_elem(&element);
        }
        self.window_index.remove(&window);
        self.active_resizes.remove(&window);
        self.drop_window_animation(window);
        self.last_configured_size.remove(&window);
        self.clear_seat_focus_if_focused(window);
        self.close_foreign_toplevel(window);
        self.push_command(crate::backend::wayland::commands::WmCommand::RequestSpaceSync);
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
