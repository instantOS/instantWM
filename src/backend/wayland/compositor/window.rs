//! Window management operations for WaylandState.
//!
//! This module contains all window-related methods on WaylandState,
//! including mapping, unmapping, resizing, focusing, and closing windows.

use std::time::{Duration, Instant};

use smithay::desktop::Window;
use smithay::utils::SERIAL_COUNTER;
use smithay::utils::{Logical, Point};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::shell::xdg::ToplevelSurface;

use crate::types::{Rect, WindowId};

use super::state::{WaylandState, WindowIdMarker};
use super::KeyboardFocusTarget;

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

        self.space.map_element(window.clone(), (0, 0), true);
        self.window_index.insert(window_id, window.clone());
        self.ensure_client_for_window(window_id);

        if let Some(title) = self.window_title(window_id) {
            if let Some(g) = self.globals_mut() {
                if let Some(client) = g.clients.get_mut(&window_id) {
                    client.name = title;
                }
            }
        }

        if let Some(toplevel) = window.toplevel() {
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
            if let Some(monitor_id) = g.clients.get(&window_id).map(|c| c.monitor_id) {
                if let Some(mon) = g.monitor_mut(monitor_id) {
                    mon.sel = Some(window_id);
                }
            }
            g.layout_dirty = true;
            g.space_dirty = true;
        }
        self.set_focus(window_id);
        window_id
    }

    /// Resize a window to the given rectangle.
    pub fn resize_window(&mut self, window: WindowId, rect: Rect) {
        if let Some(element) = self.find_window(window).cloned() {
            let border_width = self
                .globals()
                .and_then(|g| g.clients.get(&window).map(|c| c.border_width))
                .unwrap_or(0);
            let remap_immediately = self.interactive_motion_active();
            self.set_window_target_location(
                window,
                element.clone(),
                Point::from((rect.x + border_width, rect.y + border_width)),
                remap_immediately,
            );
            if let Some(_) = element.toplevel() {
                let target = (rect.w.max(1), rect.h.max(1));
                let size =
                    smithay::utils::Size::<i32, smithay::utils::Logical>::new(target.0, target.1);
                self.send_toplevel_configure(&element, Some(size));
                self.last_configured_size.insert(window, target);
            }
        }
    }

    /// Raise a window to the top of the stack.
    pub fn raise_window(&mut self, window: WindowId) {
        if let Some(element) = self.find_window(window).cloned() {
            self.space.raise_element(&element, true);
            if element.set_activated(true) {
                self.send_toplevel_configure(&element, None);
            }
        }
        self.raise_unmanaged_x11_windows();
    }

    /// Restack windows in the given order.
    pub fn restack(&mut self, windows: &[WindowId]) {
        for window in windows.iter() {
            if let Some(element) = self.find_window(*window).cloned() {
                //TODO: this has interactive false. Should that be the case even for the focussed client?
                self.space.raise_element(&element, false);
            }
        }
        self.raise_unmanaged_x11_windows();
    }

    /// Set focus to the given window.
    pub fn set_focus(&mut self, window: WindowId) {
        let serial = SERIAL_COUNTER.next_serial();
        let focus_window = self.find_window(window).cloned();

        // If the window doesn't exist in our index, don't leave stale state.
        if focus_window.is_none() && !self.window_index.contains_key(&window) {
            return;
        }

        let focus = focus_window.clone().map(KeyboardFocusTarget::Window);

        if let Some(old_id) = self.focused_window {
            if old_id != window {
                if let Some(old_window) = self.window_index.get(&old_id).cloned() {
                    if old_window.set_activated(false) {
                        self.send_toplevel_configure(&old_window, None);
                    }
                }
            }
        }
        if let Some(new_window) = focus_window {
            if new_window.set_activated(true) {
                self.send_toplevel_configure(&new_window, None);
            }
        }
        self.focused_window = Some(window);

        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, focus, serial);
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

    /// Map a window (make it visible).
    pub fn map_window(&mut self, window: WindowId) {
        // Get the location from the space if the element is already mapped,
        // otherwise use the client's stored geometry to avoid animating from (0,0)
        let is_already_mapped = self
            .find_window(window)
            .is_some_and(|w| self.space.elements().any(|e| e == w));

        //TODO: what are we doing here? Document or fix, it looks like we're
        //re-mapping an already-mapped window
        if is_already_mapped {
            if let Some(element) = self.find_window(window).cloned() {
                let loc = self
                    .space
                    .element_location(&element)
                    .or_else(|| {
                        self.globals()
                            .and_then(|g| g.clients.get(&window))
                            .map(|c| Point::from((c.geo.x, c.geo.y)))
                    })
                    .unwrap_or((0, 0).into());
                self.window_animations.remove(&window);
                self.space.map_element(element, loc, false);
                return;
            }
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
                if self.focused_window == Some(window) {
                    self.set_focus(window);
                }
            }
        }
    }

    /// Unmap a window (hide it).
    pub fn unmap_window(&mut self, window: WindowId) {
        if let Some(element) = self.window_index.get(&window).cloned() {
            self.space.unmap_elem(&element);
        }
        self.window_animations.remove(&window);
        self.last_configured_size.remove(&window);
        self.clear_keyboard_focus_if_focused(window);
        if let Some(g) = self.globals_mut() {
            g.layout_dirty = true;
            g.space_dirty = true;
        }
    }

    /// Remove all tracking for a window.
    pub(super) fn remove_window_tracking(&mut self, window: WindowId) {
        if let Some(element) = self.window_index.get(&window).cloned() {
            self.space.unmap_elem(&element);
        }
        self.window_index.remove(&window);
        self.window_animations.remove(&window);
        self.last_configured_size.remove(&window);
        self.clear_keyboard_focus_if_focused(window);
        if let Some(g) = self.globals_mut() {
            g.layout_dirty = true;
            g.space_dirty = true;
        }
    }

    /// Check whether the Smithay keyboard seat is currently focused on the
    /// X11 surface with the given `window_id`.
    pub(super) fn is_x11_surface_focused(&self, window_id: u32) -> bool {
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

    /// Explicitly clear keyboard focus on the Smithay seat so that the
    /// seat is not left pointing at a dead / dying surface.
    pub(crate) fn clear_keyboard_focus(&mut self) {
        self.focused_window = None;
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, None::<KeyboardFocusTarget>, serial);
        }
    }

    /// Clear keyboard focus if the given window is currently focused.
    /// Used when a window is unmapped or removed to avoid leaving the
    /// keyboard seat pointing at a dead surface.
    fn clear_keyboard_focus_if_focused(&mut self, window: WindowId) {
        if self.focused_window == Some(window) {
            self.clear_keyboard_focus();
        }
    }

    /// Send a configure event to a toplevel surface with the specified size.
    /// This is a helper to avoid repeating the same configure pattern.
    pub(super) fn send_toplevel_configure(
        &self,
        window: &Window,
        size: Option<smithay::utils::Size<i32, smithay::utils::Logical>>,
    ) {
        if let Some(toplevel) = window.toplevel() {
            if let Some(size) = size {
                toplevel.with_pending_state(|state| {
                    state.size = Some(size);
                });
            }
            toplevel.send_pending_configure();
        }
    }

    /// Restore focus after an overlay (e.g., dmenu) is closed.
    pub(super) fn restore_focus_after_overlay(&mut self) {
        let target = self
            .globals()
            .and_then(|g| g.selected_win())
            .filter(|w| self.window_index.contains_key(w));
        if let Some(win) = target {
            self.set_focus(win);
        } else {
            // No valid target — explicitly clear keyboard focus so the seat
            // doesn't keep pointing at the dead overlay surface.  Without
            // this, WM shortcuts stay suppressed (the input handler sees an
            // overlay as the current focus and blocks keybindings).
            self.clear_keyboard_focus();
        }
    }

    /// Request the compositor to warp the pointer to `(x, y)` in logical
    /// screen coordinates.  The warp is deferred until the next event-loop
    /// tick so that the pointer handle and the caller's `pointer_location`
    /// variable can both be updated consistently.
    pub fn request_warp(&mut self, x: f64, y: f64) {
        self.pending_warp = Some(Point::from((x, y)));
    }

    /// Consume and return the pending warp target, if any.
    pub fn take_pending_warp(&mut self) -> Option<Point<f64, Logical>> {
        self.pending_warp.take()
    }

    pub(super) fn raise_unmanaged_x11_windows(&mut self) {
        let overlays: Vec<_> = self
            .space
            .elements()
            .filter(|w| match w.user_data().get::<WindowIdMarker>() {
                Some(m) => m.is_overlay,
                None => w.x11_surface().is_some(),
            })
            .cloned()
            .collect();
        for w in overlays {
            self.space.raise_element(&w, true);
        }
    }

    /// Collect all overlay/unmanaged windows (dmenu, override-redirect popups,
    /// etc.) that should be rendered above the bar but below the cursor.
    ///
    /// Returns `(window, physical_location)` pairs ready for `AsRenderElements`.
    ///
    /// # Why this exists
    ///
    /// The bar is rendered as a `custom_element` which sits *above* every
    /// element in `self.space` (Smithay's `render_output` prepends custom
    /// elements before space elements in the front-to-back list).  Overlay
    /// windows such as dmenu live inside the space and are therefore drawn
    /// *beneath* the bar, which makes them invisible.
    ///
    /// The fix is to pull those windows out of the space's render contribution
    /// and re-emit them as custom elements inserted between the cursor and the
    /// bar.  The space still maps/tracks them for hit-testing and protocol
    /// bookkeeping; we just override where in the z-stack they are drawn.
    pub fn overlay_windows_for_render(
        &self,
        x_offset: i32,
    ) -> Vec<(Window, Point<i32, smithay::utils::Physical>)> {
        use smithay::utils::Physical;

        self.space
            .elements()
            .filter(|w| match w.user_data().get::<WindowIdMarker>() {
                Some(m) => m.is_overlay,
                // Windows with no marker are unmananged override-redirect X11
                // surfaces mapped directly (e.g. via mapped_override_redirect_window).
                None => w.x11_surface().is_some(),
            })
            .filter_map(|w| {
                let loc = self.space.element_location(w)?;
                // Translate from global compositor coordinates to the
                // per-output local coordinate space, then convert to physical
                // pixels (scale = 1 throughout, so this is a no-op numerically
                // but keeps the type system happy).
                let phys = Point::<i32, Physical>::from((loc.x - x_offset, loc.y));
                Some((w.clone(), phys))
            })
            .collect()
    }

    /// Check if a window exists.
    pub fn window_exists(&self, window: WindowId) -> bool {
        self.window_index.contains_key(&window)
    }

    /// Allocate a new window ID.
    pub(super) fn alloc_window_id(&mut self) -> WindowId {
        loop {
            let id = self.next_window_id;
            self.next_window_id = self.next_window_id.wrapping_add(1).max(1);
            let window_id = WindowId::from(id);
            if !self.window_index.contains_key(&window_id) {
                return window_id;
            }
        }
    }

    /// Get the surface under a given point.
    pub fn surface_under_pointer(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<i32, Logical>,
    )> {
        use smithay::desktop::WindowSurfaceType;

        for window in self.space.elements().rev() {
            let Some(loc) = self.space.element_location(window) else {
                continue;
            };
            let geo_offset = window.geometry().loc;
            let surface_origin = loc - geo_offset;
            if let Some(result) =
                window.surface_under(point - surface_origin.to_f64(), WindowSurfaceType::POPUP)
            {
                return Some((result.0, result.1 + surface_origin));
            }
        }
        if let Some((window, loc)) = self.space.element_under(point) {
            if let Some(result) = window.surface_under(point - loc.to_f64(), WindowSurfaceType::ALL)
            {
                return Some((result.0, result.1 + loc));
            }
        }
        None
    }

    /// Get the layer surface under a given point.
    pub fn layer_surface_under_pointer(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<i32, Logical>,
    )> {
        use smithay::desktop::{layer_map_for_output, WindowSurfaceType};

        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        for output in outputs.iter().rev() {
            let map = layer_map_for_output(output);
            for layer in map.layers().rev() {
                let Some(geo) = map.layer_geometry(layer) else {
                    continue;
                };
                let rel = point - geo.loc.to_f64();
                if let Some((surface, loc)) = layer.surface_under(rel, WindowSurfaceType::ALL) {
                    return Some((surface, loc + geo.loc));
                }
            }
        }
        None
    }

    /// Get the layer surface that should receive keyboard focus.
    pub fn keyboard_focus_layer_surface(
        &self,
    ) -> Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface> {
        use smithay::desktop::layer_map_for_output;

        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        for output in outputs.iter().rev() {
            let map = layer_map_for_output(output);
            for layer in map.layers().rev() {
                if layer.can_receive_keyboard_focus() {
                    return Some(layer.wl_surface().clone());
                }
            }
        }
        None
    }

    /// Get the title of a window.
    pub fn window_title(&self, window: WindowId) -> Option<String> {
        let element = self.window_index.get(&window)?;
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

    /// Find a window by ID.
    pub(super) fn find_window(&self, window: WindowId) -> Option<&Window> {
        self.window_index.get(&window)
    }

    /// Ensure a client exists for the given window.
    pub(super) fn ensure_client_for_window(&mut self, window: WindowId) {
        let Some(g) = self.globals_mut() else {
            return;
        };
        if g.clients.contains_key(&window) {
            return;
        }

        let monitor_id = g.selected_monitor_id();
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
        c.tags = crate::client::initial_tags_for_monitor(g, c.monitor_id);
        g.clients.insert(window, c);
        g.attach(window);
        g.attach_stack(window);
    }

    /// Get the window ID for a toplevel surface.
    pub(super) fn window_id_for_toplevel(&self, surface: &ToplevelSurface) -> Option<WindowId> {
        let wl_surface = surface.wl_surface();
        self.window_index
            .values()
            .find(|w| w.wl_surface().as_deref() == Some(wl_surface))
            .and_then(|w| w.user_data().get::<WindowIdMarker>().map(|m| m.id))
    }

    /// Get the window ID for an X11 surface.
    pub(super) fn window_id_for_x11_surface(
        &self,
        surface: &smithay::xwayland::X11Surface,
    ) -> Option<WindowId> {
        self.window_index
            .values()
            .find(|w| w.x11_surface().is_some_and(|x11| x11 == surface))
            .and_then(|w| w.user_data().get::<WindowIdMarker>().map(|m| m.id))
    }

    /// Get the window ID for a surface.
    pub(crate) fn window_id_for_surface(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<WindowId> {
        use smithay::desktop::WindowSurfaceType;

        self.window_index.iter().find_map(|(win, window)| {
            if window.wl_surface().as_deref() == Some(surface) {
                return Some(*win);
            }

            let owns_surface = window
                .surface_under((0.0, 0.0), WindowSurfaceType::ALL)
                .map(|(hit_surface, _)| hit_surface == *surface)
                .unwrap_or(false);
            if owns_surface {
                Some(*win)
            } else {
                None
            }
        })
    }

    /// Get the currently focused window ID, if any.
    pub fn focused_window(&self) -> Option<WindowId> {
        self.focused_window
    }

    pub(super) const MIN_WL_DIM: i32 = 64;

    fn animations_enabled(&self) -> bool {
        self.globals().map(|g| g.animated).unwrap_or(false)
    }

    fn interactive_motion_active(&self) -> bool {
        self.globals()
            .map(|g| g.drag.title.active || g.drag.title.dragging || g.drag.hover_resize.active)
            .unwrap_or(false)
    }

    pub(super) fn set_window_target_location(
        &mut self,
        window_id: WindowId,
        element: Window,
        target: Point<i32, Logical>,
        remap: bool,
    ) {
        // Use the client's stored geometry as the authoritative current position
        // to avoid animating from stale locations after map/unmap cycles.
        // This is especially important on tag switches where windows are
        // unmapped and then mapped again at their existing geometry.
        // Note: target includes border width offset, so we add it to current too.
        let current = self
            .globals()
            .and_then(|g| {
                g.clients
                    .get(&window_id)
                    .map(|c| Point::from((c.geo.x + c.border_width, c.geo.y + c.border_width)))
            })
            .or_else(|| self.space.element_location(&element))
            .unwrap_or(target);
        if !self.animations_enabled() || remap || current == target {
            self.window_animations.remove(&window_id);
            self.space.map_element(element, target, remap);
            return;
        }

        if self
            .window_animations
            .get(&window_id)
            .is_some_and(|anim| anim.to == target)
        {
            return;
        }

        self.window_animations.insert(
            window_id,
            WaylandWindowAnimation {
                from: current,
                to: target,
                started_at: Instant::now(),
                duration: Duration::from_millis(90),
            },
        );
    }

    /// Tick all active window animations.
    pub fn tick_window_animations(&mut self) {
        if self.window_animations.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut updates: Vec<(WindowId, Point<i32, Logical>, bool)> = Vec::new();
        for (win, anim) in &self.window_animations {
            let elapsed = now.saturating_duration_since(anim.started_at);
            let raw_t = (elapsed.as_secs_f64() / anim.duration.as_secs_f64()).clamp(0.0, 1.0);
            let t = crate::animation::ease_out_cubic(raw_t);
            let x = anim.from.x + ((anim.to.x - anim.from.x) as f64 * t).round() as i32;
            let y = anim.from.y + ((anim.to.y - anim.from.y) as f64 * t).round() as i32;
            updates.push((*win, Point::from((x, y)), raw_t >= 1.0));
        }

        let mut finished: Vec<WindowId> = Vec::new();
        for (win, loc, done) in updates {
            if let Some(element) = self.find_window(win).cloned() {
                self.space.map_element(element, loc, false);
            } else {
                finished.push(win);
                continue;
            }
            if done {
                finished.push(win);
            }
        }
        for win in finished {
            self.window_animations.remove(&win);
        }
    }

    /// Check if there are active window animations.
    pub fn has_active_window_animations(&self) -> bool {
        !self.window_animations.is_empty()
    }
}

/// Window animation state.
#[derive(Debug, Clone, Copy)]
pub struct WaylandWindowAnimation {
    from: Point<i32, Logical>,
    to: Point<i32, Logical>,
    started_at: Instant,
    duration: Duration,
}
