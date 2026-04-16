use smithay::desktop::Window;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as ToplevelState;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::shell::xdg::ToplevelSurface;

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::compositor::state::{PendingLaunchContextMarker, WindowIdMarker};
use crate::types::{Rect, WindowId};

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
    }

    /// Get properties for rule matching.
    pub fn window_properties(&self, window: WindowId) -> crate::client::WindowProperties {
        crate::client::WindowProperties {
            class: self.window_app_id(window).unwrap_or_default(),
            instance: String::new(), // Wayland doesn't really have instance vs class
            title: self.window_title(window).unwrap_or_default(),
        }
    }

    /// Ensure a client exists for the given window.
    pub(crate) fn ensure_client_for_window(&mut self, window: WindowId) {
        if self
            .globals()
            .is_some_and(|g| g.clients.contains_key(&window))
        {
            return;
        }

        let props = self.window_properties(window);
        let x11_launch_ids = self
            .find_window(window)
            .and_then(|element| element.x11_surface())
            .map(|x11| (x11.pid(), x11.startup_id()));
        let launch_context = x11_launch_ids
            .and_then(|(pid, startup_id)| {
                self.globals_mut()
                    .and_then(|g| crate::client::take_pending_launch(g, pid, startup_id.as_deref()))
            })
            .or_else(|| {
                self.find_window(window)
                    .and_then(|element| element.wl_surface())
                    .and_then(|wl_surface| {
                        smithay::wayland::compositor::with_states(&wl_surface, |states| {
                            states
                                .data_map
                                .get::<PendingLaunchContextMarker>()
                                .map(|marker| marker.context)
                        })
                    })
            });

        let Some(g) = self.globals_mut() else {
            return;
        };
        let monitor_id = launch_context
            .map(|ctx| ctx.monitor_id)
            .unwrap_or_else(|| g.selected_monitor_id());
        let (base_w, base_h) = g
            .monitor(monitor_id)
            .map(|m| {
                (
                    m.work_rect.w.max(Self::MIN_WL_DIM),
                    m.work_rect.h.max(Self::MIN_WL_DIM),
                )
            })
            .unwrap_or((
                g.cfg.screen_width.max(Self::MIN_WL_DIM),
                g.cfg.screen_height.max(Self::MIN_WL_DIM),
            ));
        let geo = Rect {
            x: 0,
            y: 0,
            w: base_w,
            h: base_h,
        };

        let mut c = crate::types::Client::default();
        c.win = window;
        c.geo = geo;
        c.old_geo = geo;
        c.float_geo = geo;
        c.border_width = g.cfg.border_width_px;
        c.old_border_width = g.cfg.border_width_px;
        c.monitor_id = monitor_id;
        c.set_tag_mask(
            launch_context
                .map(|ctx| ctx.tags)
                .unwrap_or_else(|| crate::client::initial_tags_for_monitor(g, c.monitor_id)),
        );

        g.clients.insert(window, c);
        crate::client::apply_rules(g, window, &props, launch_context);

        g.attach(window);
        g.attach_z_order_top(window);
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
            // Instead, we check if the surface is part of this window's surface hierarchy.
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
    pub(crate) fn send_toplevel_configure(
        &self,
        window: &Window,
        size: Option<smithay::utils::Size<i32, smithay::utils::Logical>>,
    ) {
        if let Some(toplevel) = window.toplevel() {
            let is_resizing = window
                .user_data()
                .get::<WindowIdMarker>()
                .is_some_and(|marker| self.active_resizes.contains(&marker.id));
            let is_fullscreen = window
                .user_data()
                .get::<WindowIdMarker>()
                .and_then(|marker| {
                    self.globals()
                        .and_then(|g| g.clients.get(&marker.id).map(|c| c.is_fullscreen))
                })
                .unwrap_or(false);
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
            });
            toplevel.send_pending_configure();
        }
    }
}
