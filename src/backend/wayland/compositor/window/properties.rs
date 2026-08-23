use smithay::desktop::Window;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as ToplevelState;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::shell::xdg::ToplevelSurface;

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::compositor::state::WindowIdMarker;
use crate::types::WindowId;

impl WaylandState {
    pub(crate) const MIN_WL_DIM: i32 = 64;

    /// Get the title of a window.
    ///
    /// For XWayland (X11) surfaces the title comes from the X11 property;
    /// for native Wayland toplevels it comes from `xdg_toplevel::set_title`.
    pub fn window_title(&self, window: WindowId) -> Option<String> {
        let element = self.window_index.get(&window)?;

        if let Some(x11) = element.x11_surface() {
            return Some(x11.title());
        }

        let wl_surface = element.wl_surface()?;
        smithay::wayland::compositor::with_states(&wl_surface, |states| {
            states
                .data_map
                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()?
                .lock()
                .ok()?
                .title
                .clone()
        })
    }

    /// Get the app_id (desktop file ID) of a window.
    pub fn window_app_id(&self, window: WindowId) -> Option<String> {
        let element = self.window_index.get(&window)?;

        if let Some(x11) = element.x11_surface() {
            let wm_class = x11.class();
            return Some(wm_class);
        }

        let wl_surface = element.wl_surface()?;
        smithay::wayland::compositor::with_states(&wl_surface, |states| {
            states
                .data_map
                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()?
                .lock()
                .ok()?
                .app_id
                .clone()
        })
    }

    /// Return the protocol/surface family that owns this window.
    pub(crate) fn window_protocol(&self, window: WindowId) -> crate::backend::WindowProtocol {
        let Some(element) = self.window_index.get(&window) else {
            return crate::backend::WindowProtocol::Unknown;
        };

        if element.x11_surface().is_some() {
            crate::backend::WindowProtocol::XWayland
        } else if element.wl_surface().is_some() {
            crate::backend::WindowProtocol::Wayland
        } else {
            crate::backend::WindowProtocol::Unknown
        }
    }

    /// Create a foreign toplevel handle for a window.
    pub(crate) fn create_foreign_toplevel(&mut self, window: WindowId) {
        let title = self.window_title(window).unwrap_or_default();
        let app_id = self.window_app_id(window).unwrap_or_default();
        let handle = self
            .foreign_toplevel_list_state
            .new_toplevel::<Self>(title, app_id);
        self.foreign_toplevel_handles.insert(window, handle);
    }

    /// Update the foreign toplevel handle for a window (title/app_id changed).
    pub fn update_foreign_toplevel(&mut self, window: WindowId) {
        let Some(handle) = self.foreign_toplevel_handles.get(&window) else {
            return;
        };
        if let Some(title) = self.window_title(window) {
            handle.send_title(&title);
        }
        if let Some(app_id) = self.window_app_id(window) {
            handle.send_app_id(&app_id);
        }
        handle.send_done();
    }

    /// Close the foreign toplevel handle for a window.
    pub(crate) fn close_foreign_toplevel(&mut self, window: WindowId) {
        if let Some(handle) = self.foreign_toplevel_handles.remove(&window) {
            self.foreign_toplevel_list_state.remove_toplevel(&handle);
        }
        self.foreign_toplevel_management_state
            .remove_toplevel::<Self>(window);
    }

    /// Compute the protocol-visible presentation of a managed window for
    /// wlr-foreign-toplevel-management. `None` while the window is not yet in
    /// the managed model (freshly registered surface awaiting its map work).
    pub(crate) fn foreign_toplevel_snapshot(
        &self,
        window: WindowId,
    ) -> Option<crate::backend::wayland::compositor::protocols::foreign_toplevel::ToplevelSnapshot>
    {
        use crate::backend::wayland::compositor::protocols::foreign_toplevel::ToplevelSnapshot;

        let core = self.globals()?;
        let client = core.model.client(window)?;
        Some(ToplevelSnapshot {
            title: self.window_title(window).unwrap_or_default(),
            app_id: self.window_app_id(window).unwrap_or_default(),
            activated: core.model.selected_win() == Some(window),
            minimized: client.is_minimized(),
            maximized: client.mode().is_maximized(),
            fullscreen: client.mode().is_true_fullscreen(),
            parent: client.transient_for,
            outputs: self
                .find_window(window)
                .map(|element| self.outputs_for_window_geometry(element))
                .unwrap_or_default(),
        })
    }

    /// Push the current presentation of one window to managing clients
    /// (advertises it on first sight; diffs afterwards). Cheap when nothing
    /// changed.
    pub fn refresh_foreign_toplevel(&mut self, window: WindowId) {
        if !self.foreign_toplevel_handles.contains_key(&window) {
            return;
        }
        if let Some(snapshot) = self.foreign_toplevel_snapshot(window) {
            self.foreign_toplevel_management_state
                .sync_toplevel::<Self>(window, &snapshot);
        }
    }

    /// Refresh every managed window's advertised presentation.
    pub fn refresh_all_foreign_toplevels(&mut self) {
        let windows: Vec<WindowId> = self.foreign_toplevel_handles.keys().copied().collect();
        for window in windows {
            self.refresh_foreign_toplevel(window);
        }
    }

    /// Get properties for rule matching.
    pub fn window_properties(&self, window: WindowId) -> crate::client::WindowProperties {
        crate::client::WindowProperties {
            class: self.window_app_id(window).unwrap_or_default(),
            instance: String::new(), // Wayland doesn't really have instance vs class
            title: self.window_title(window).unwrap_or_default(),
            size_hints: self.native_size_hints(window),
        }
    }

    fn native_size_hints(&self, window: WindowId) -> Option<crate::types::SizeHints> {
        let element = self.window_index.get(&window)?;
        if element.x11_surface().is_some() {
            return None;
        }
        let surface = element.wl_surface()?;
        smithay::wayland::compositor::with_states(&surface, |states| {
            use smithay::wayland::shell::xdg::SurfaceCachedState;
            let mut guard = states.cached_state.get::<SurfaceCachedState>();
            let current = *guard.current();
            Some(crate::types::SizeHints {
                min_width: current.min_size.w.max(0),
                min_height: current.min_size.h.max(0),
                max_width: current.max_size.w.max(0),
                max_height: current.max_size.h.max(0),
                ..Default::default()
            })
        })
    }

    pub(crate) fn native_size_hints_changed(&mut self, window: WindowId) -> bool {
        let Some(current) = self.native_size_hints(window) else {
            return false;
        };
        self.native_size_hints.insert(window, current) != Some(current)
    }

    /// Get the window ID for a toplevel surface.
    pub(crate) fn window_id_for_toplevel(&self, surface: &ToplevelSurface) -> Option<WindowId> {
        let wl_surface = surface.wl_surface();
        self.window_index
            .values()
            .find(|w| w.wl_surface().as_deref() == Some(wl_surface))
            .and_then(|w| w.user_data().get::<WindowIdMarker>().map(|m| m.id))
    }

    /// Get the window ID for an X11 surface.
    pub(crate) fn window_id_for_x11_surface(
        &self,
        surface: &smithay::xwayland::X11Surface,
    ) -> Option<WindowId> {
        self.window_index
            .values()
            .find(|w| w.x11_surface().is_some_and(|x11| x11 == surface))
            .and_then(|w| w.user_data().get::<WindowIdMarker>().map(|m| m.id))
    }

    pub(crate) fn window_id_for_x11_window(&self, window: u32) -> Option<WindowId> {
        self.window_index
            .values()
            .find(|w| w.x11_surface().is_some_and(|x11| x11.window_id() == window))
            .and_then(|w| w.user_data().get::<WindowIdMarker>().map(|m| m.id))
    }

    /// Get the window ID for a surface.
    pub(crate) fn window_id_for_surface(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<WindowId> {
        self.window_index.iter().find_map(|(win, window)| {
            if window.wl_surface().as_deref() == Some(surface) {
                return Some(*win);
            }

            // A window owns a surface if it's anywhere in its subsurface or popup tree.
            // Using a large negative offset for surface_under is not reliable.
            // Instead, we check if the surface's states data map contains our window id.
            let mut owns_surface = false;
            window.with_surfaces(|s, _| {
                if s == surface {
                    owns_surface = true;
                }
            });

            if owns_surface { Some(*win) } else { None }
        })
    }

    /// Send a configure event to a toplevel surface with the specified size.
    /// This is a helper to avoid repeating the same configure pattern.
    ///
    /// Returns the serial of the configure that was actually sent. Every
    /// size-bearing configure is registered here as the window's latest
    /// outstanding request, so commits acknowledging an older serial are
    /// always classified against it.
    pub(crate) fn send_toplevel_configure(
        &mut self,
        window: &Window,
        size: Option<smithay::utils::Size<i32, smithay::utils::Logical>>,
    ) -> Option<smithay::utils::Serial> {
        let toplevel = window.toplevel()?;
        let is_resizing = window
            .user_data()
            .get::<WindowIdMarker>()
            .is_some_and(|marker| self.active_resizes.contains(&marker.id));
        let presentation = window
            .user_data()
            .get::<WindowIdMarker>()
            .and_then(|marker| {
                self.globals().and_then(|state| {
                    state.model.client(marker.id).map(|client| {
                        (
                            client.mode().is_fullscreen(),
                            state
                                .model
                                .client_protocol_maximized(marker.id)
                                .unwrap_or(false),
                        )
                    })
                })
            });
        let is_fullscreen = presentation.is_some_and(|state| state.0);
        let is_maximized = presentation.is_some_and(|state| state.1);
        toplevel.with_pending_state(|state| {
            if let Some(size) = size {
                state.size = Some(size);
            }
            if is_resizing {
                state.states.set(ToplevelState::Resizing);
            } else {
                state.states.unset(ToplevelState::Resizing);
            }
            if is_fullscreen {
                state.states.set(ToplevelState::Fullscreen);
            } else {
                state.states.unset(ToplevelState::Fullscreen);
            }
            if is_maximized {
                state.states.set(ToplevelState::Maximized);
            } else {
                state.states.unset(ToplevelState::Maximized);
            }
        });
        let serial = toplevel.send_pending_configure();
        if let (Some(serial), Some(_), Some(marker)) =
            (serial, size, window.user_data().get::<WindowIdMarker>())
        {
            self.pending_size_configure.insert(marker.id, serial);
        }
        serial
    }

    /// Re-project the authoritative WM presentation to the surface protocol.
    ///
    /// This is needed for state-only transitions whose resulting geometry may
    /// equal the previous geometry. Relying on a resize to incidentally send a
    /// configure would otherwise allow the protocol and model to drift.
    pub(crate) fn sync_window_presentation(&mut self, win: WindowId) {
        let Some(window) = self.find_window(win).cloned() else {
            return;
        };
        let Some((mode, maximized)) = self.globals().and_then(|state| {
            state.model.client(win).map(|client| {
                (
                    client.mode(),
                    state.model.client_protocol_maximized(win).unwrap_or(false),
                )
            })
        }) else {
            return;
        };

        if let Some(surface) = window.x11_surface() {
            let _ = surface.set_maximized(maximized);
            let _ = surface.set_fullscreen(mode.is_fullscreen());
        } else {
            self.send_toplevel_configure(&window, None);
        }
    }
}
