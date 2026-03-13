use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use smithay::wayland::seat::WaylandFocus;
use smithay::{
    backend::allocator::Format,
    backend::drm::DrmNode,
    backend::egl::{EGLDevice, EGLDisplay},
    backend::renderer::gles::GlesRenderer,
    desktop::{layer_map_for_output, PopupManager, Space, Window, WindowSurfaceType},
    input::{
        keyboard::{KeyboardHandle, XkbConfig},
        pointer::PointerHandle,
        Seat, SeatState,
    },
    output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        calloop::{generic::Generic, Interest, LoopHandle, Mode, PostAction},
        wayland_server::{Display, DisplayHandle},
    },
    utils::{Logical, Physical, Point, Transform, SERIAL_COUNTER},
    wayland::{
        compositor::CompositorState,
        dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{decoration::XdgDecorationState, ToplevelSurface, XdgShellState},
        },
        shm::ShmState,
        xwayland_shell::XWaylandShellState,
    },
    xwayland::X11Wm,
};

use crate::globals::Globals;
use crate::types::{Client as WmClient, Rect, WindowId};
use crate::wm::Wm;

use super::screencopy::PendingScreencopy;
use super::KeyboardFocusTarget;

// ---------------------------------------------------------------------------
// Per-client state
// ---------------------------------------------------------------------------

/// State attached to each connected Wayland client.
///
/// Smithay requires every client inserted via `DisplayHandle::insert_client`
/// to carry a `ClientData` implementor.  The `compositor_state` field is
/// mandatory for the compositor protocol to track per-client double-buffer
/// state.
#[derive(Debug, Default)]
pub struct WaylandClientState {
    pub compositor_state: smithay::wayland::compositor::CompositorClientState,
}

impl smithay::reexports::wayland_server::backend::ClientData for WaylandClientState {
    fn initialized(&self, _client_id: smithay::reexports::wayland_server::backend::ClientId) {}
    fn disconnected(
        &self,
        _client_id: smithay::reexports::wayland_server::backend::ClientId,
        _reason: smithay::reexports::wayland_server::backend::DisconnectReason,
    ) {
    }
}

// ---------------------------------------------------------------------------
// Compositor state
// ---------------------------------------------------------------------------

/// The main Wayland compositor state.
///
/// This struct owns all Smithay protocol state objects and is the target
/// of every `delegate_*!` macro.  It also bridges into instantWM's
/// `Globals` for shared WM state (tags, clients, config, etc.).
pub struct WaylandState {
    // -- Wayland infrastructure --
    pub display_handle: DisplayHandle,

    // -- Desktop abstractions --
    pub space: Space<Window>,
    pub popups: PopupManager,

    // -- Protocol states --
    pub compositor_state: CompositorState,
    pub shm_state: ShmState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub seat_state: SeatState<WaylandState>,
    pub output_manager_state: OutputManagerState,
    pub data_device_state: DataDeviceState,
    pub xwayland_shell_state: XWaylandShellState,
    pub wlr_layer_shell_state: WlrLayerShellState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    renderer: Option<NonNull<GlesRenderer>>,

    // -- Input --
    pub seat: Seat<WaylandState>,
    pub keyboard: KeyboardHandle<WaylandState>,
    pub pointer: PointerHandle<WaylandState>,
    pub cursor_image_status: smithay::input::pointer::CursorImageStatus,
    pub cursor_icon_override: Option<smithay::input::pointer::CursorIcon>,

    // -- XWayland --
    pub xwm: Option<X11Wm>,
    pub xdisplay: Option<u32>,

    next_window_id: u32,
    wm: Option<NonNull<Wm>>,
    pub tracked_devices: Vec<smithay::reexports::input::Device>,
    pub(super) last_configured_size: HashMap<WindowId, (i32, i32)>,
    /// O(1) window lookup index containing all known windows (mapped and hidden).
    pub(super) window_index: HashMap<WindowId, Window>,
    window_animations: HashMap<WindowId, WaylandWindowAnimation>,
    /// Currently focused window for O(1) deactivate-old / activate-new.
    focused_window: Option<WindowId>,

    /// Pending screencopy frames waiting to be fulfilled during the next render.
    pub pending_screencopies: Vec<PendingScreencopy>,

    /// Pending cursor warp requested by the WM (e.g. warp-to-focus keybinding).
    /// The event loop consumes this each tick and synthesises a pointer motion.
    pub pending_warp: Option<Point<f64, Logical>>,

    /// Channel to notify the DRM backend loop of keyboard LED state changes.
    pub led_state_tx: Option<std::sync::mpsc::Sender<smithay::input::keyboard::LedState>>,
}

#[derive(Debug, Clone, Copy)]
struct WaylandWindowAnimation {
    from: Point<i32, Logical>,
    to: Point<i32, Logical>,
    started_at: Instant,
    duration: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowIdMarker {
    pub id: WindowId,
    /// Cached: true when this is an unmanaged X11 overlay (dmenu, popup, etc.).
    pub is_overlay: bool,
}

impl WaylandState {
    const MIN_WL_DIM: i32 = 64;
    /// Create a new `WaylandState` and register all Wayland globals.
    pub fn new(display: Display<WaylandState>, handle: &LoopHandle<'static, WaylandState>) -> Self {
        let dh = display.handle();

        // Insert the Wayland display as a calloop source so that protocol
        // messages from connected clients are dispatched on each loop tick.
        handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, data| {
                    let dispatch_result = catch_unwind(AssertUnwindSafe(|| unsafe {
                        display.get_mut().dispatch_clients(data)
                    }));
                    match dispatch_result {
                        Ok(Ok(_)) => {}
                        Ok(Err(err)) => {
                            log::warn!("wayland dispatch_clients error: {}", err);
                        }
                        Err(_) => {
                            log::error!(
                                "wayland client dispatch panicked (invalid client request); continuing"
                            );
                        }
                    }
                    Ok(PostAction::Continue)
                },
            )
            .expect("Failed to insert Wayland display source");

        // -- Protocol globals --
        let compositor_state = CompositorState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let xwayland_shell_state = XWaylandShellState::new::<Self>(&dh);
        let wlr_layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let dmabuf_state = DmabufState::new();

        // -- Seat (input devices) --
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "seat-0");
        let keyboard = seat
            .add_keyboard(XkbConfig::default(), 400, 25)
            .expect("Failed to add keyboard to seat");
        let pointer = seat.add_pointer();

        WaylandState {
            display_handle: dh,
            space: Space::default(),
            popups: PopupManager::default(),
            compositor_state,
            shm_state,
            xdg_shell_state,
            xdg_decoration_state,
            seat_state,
            output_manager_state,
            data_device_state,
            xwayland_shell_state,
            wlr_layer_shell_state,
            dmabuf_state,
            dmabuf_global: None,
            renderer: None,
            seat,
            keyboard,
            pointer,
            cursor_image_status: smithay::input::pointer::CursorImageStatus::default_named(),
            cursor_icon_override: None,
            xwm: None,
            xdisplay: None,
            next_window_id: 1,
            wm: None,
            tracked_devices: Vec::new(),
            last_configured_size: HashMap::new(),
            window_index: HashMap::new(),
            window_animations: HashMap::new(),
            focused_window: None,
            pending_screencopies: Vec::new(),
            pending_warp: None,
            led_state_tx: None,
        }
    }

    fn animations_enabled(&self) -> bool {
        self.globals().map(|g| g.animated).unwrap_or(false)
    }

    fn interactive_motion_active(&self) -> bool {
        self.globals()
            .map(|g| g.drag.title.active || g.drag.title.dragging || g.drag.hover_resize.active)
            .unwrap_or(false)
    }

    fn set_window_target_location(
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

    pub fn has_active_window_animations(&self) -> bool {
        !self.window_animations.is_empty()
    }

    pub fn attach_wm(&mut self, wm: &mut Wm) {
        self.wm = Some(NonNull::from(wm));
    }

    pub fn with_wm<T>(&mut self, f: impl FnOnce(&mut WaylandState, &mut Wm) -> T) -> Option<T> {
        let mut wm = self.wm?;
        Some(unsafe { f(self, wm.as_mut()) })
    }

    /// Initialise the linux-dmabuf global.
    ///
    /// When `egl_display` is provided and a render DRM node can be resolved
    /// from it, we advertise `zwp_linux_dmabuf_feedback_v1` **v4** which
    /// includes the device node identifier.  GPU-accelerated clients (kitty,
    /// wlroots apps, etc.) use this to discover which DRM device to open for
    /// dmabuf allocation and to choose zero-copy import paths — without it
    /// Mesa/EGL falls back to software rendering and emits warnings like
    /// "failed to get driver name" / "failed to retrieve device information".
    ///
    /// Falls back to the plain v3 global (formats only, no device) when no
    /// EGL display is given or the node cannot be resolved.
    pub fn init_dmabuf_global(&mut self, formats: Vec<Format>, egl_display: Option<&EGLDisplay>) {
        if self.dmabuf_global.is_some() {
            return;
        }

        // Attempt to get the render DrmNode from the EGL display so we can
        // advertise zwp_linux_dmabuf_feedback_v1 v4 with a proper device id.
        let render_node: Option<DrmNode> = egl_display.and_then(|display| {
            EGLDevice::device_for_display(display)
                .map_err(|err| {
                    log::warn!("dmabuf: failed to query EGLDevice for display: {err}");
                })
                .ok()
                .and_then(|dev| {
                    dev.try_get_render_node()
                        .map_err(|err| {
                            log::warn!("dmabuf: failed to query render node from EGLDevice: {err}");
                        })
                        .ok()
                        .flatten()
                })
        });

        self.dmabuf_global = Some(if let Some(node) = render_node {
            log::info!("dmabuf: advertising zwp_linux_dmabuf_feedback_v1 v4 on node {node:?}");
            let feedback = DmabufFeedbackBuilder::new(node.dev_id(), formats)
                .build()
                .expect("DmabufFeedbackBuilder::build");
            self.dmabuf_state
                .create_global_with_default_feedback::<Self>(&self.display_handle, &feedback)
        } else {
            log::info!("dmabuf: no render node available, falling back to zwp_linux_dmabuf_v1 v3");
            self.dmabuf_state
                .create_global::<Self>(&self.display_handle, formats)
        });
    }

    pub fn attach_renderer(&mut self, renderer: &mut GlesRenderer) {
        self.renderer = Some(NonNull::from(renderer));
        // Bind the compositor's Wayland display to the EGL display.  This
        // enables the legacy EGL_WL_bind_wayland_display / wl_drm path that
        // Mesa falls back to when zwp_linux_dmabuf_feedback_v1 v4 is
        // unavailable.  Together with the v4 dmabuf feedback we advertise in
        // init_dmabuf_global this ensures GPU clients like kitty never need
        // to resort to software rendering.
        #[cfg(feature = "use_system_lib")]
        {
            match renderer.bind_wl_display(&self.display_handle) {
                Ok(()) => log::info!("EGL wl_drm hardware-acceleration enabled"),
                Err(err) => log::debug!(
                    "EGL wl_drm not available ({}); dmabuf v4 will be used instead",
                    err
                ),
            }
        }
    }

    pub(super) fn renderer_mut(&mut self) -> Option<&mut GlesRenderer> {
        self.renderer.map(|mut p| unsafe { p.as_mut() })
    }

    #[inline]
    fn globals(&self) -> Option<&Globals> {
        self.wm.map(|p: NonNull<Wm>| unsafe { &p.as_ref().g })
    }

    #[inline]
    pub(super) fn globals_mut(&mut self) -> Option<&mut Globals> {
        self.wm
            .map(|mut p: NonNull<Wm>| unsafe { &mut p.as_mut().g })
    }

    /// Create and register a default output.
    pub fn create_output(&mut self, name: &str, width: i32, height: i32) -> Output {
        let safe_width = width.max(Self::MIN_WL_DIM);
        let safe_height = height.max(Self::MIN_WL_DIM);
        let output = Output::new(
            name.to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "instantOS".into(),
                model: "instantWM".into(),
            },
        );

        let mode = OutputMode {
            size: (safe_width, safe_height).into(),
            refresh: 60_000,
        };

        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(Scale::Integer(1)),
            Some((0, 0).into()),
        );
        output.set_preferred(mode);

        let _global = output.create_global::<WaylandState>(&self.display_handle);
        self.space.map_output(&output, (0, 0));

        output
    }

    pub fn list_displays(&self) -> Vec<String> {
        self.space.outputs().map(|o| o.name()).collect()
    }

    pub fn list_display_modes(&self, display: &str) -> Vec<String> {
        let mut result = Vec::new();
        if let Some(output) = self.space.outputs().find(|o| o.name() == display) {
            for mode in output.modes() {
                result.push(format!(
                    "{}x{}@{}",
                    mode.size.w,
                    mode.size.h,
                    mode.refresh as f64 / 1000.0
                ));
            }
        }
        result
    }

    pub fn set_display_mode(&mut self, display: &str, width: i32, height: i32) {
        if let Some(output) = self.space.outputs().find(|o| o.name() == display).cloned() {
            if let Some(mode) = output
                .modes()
                .into_iter()
                .find(|m| m.size.w == width && m.size.h == height)
            {
                output.change_current_state(Some(mode), None, None, None);
            }
        }
    }

    pub fn set_output_config(
        &mut self,
        display: &str,
        config: &crate::config::config_toml::MonitorConfig,
    ) {
        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        for output in outputs {
            if display != "*" && output.name() != display {
                continue;
            }

            let mut current_mode = output.current_mode();
            let mut current_scale = output.current_scale();
            let mut current_transform = output.current_transform();
            let mut current_location = self
                .space
                .output_geometry(&output)
                .map(|g| g.loc)
                .unwrap_or_default();

            if let Some(ref res) = config.resolution {
                if let Some((w_str, h_str)) = res.split_once('x') {
                    if let (Ok(w), Ok(h)) = (w_str.parse::<i32>(), h_str.parse::<i32>()) {
                        if let Some(mode) = output.modes().into_iter().find(|m| {
                            m.size.w == w
                                && m.size.h == h
                                && config
                                    .refresh_rate
                                    .map(|r| (m.refresh as f32 / 1000.0 - r).abs() < 0.1)
                                    .unwrap_or(true)
                        }) {
                            current_mode = Some(mode);
                        }
                    }
                }
            }

            if let Some(scale) = config.scale {
                current_scale = Scale::Fractional(scale as f64);
            }

            if let Some(ref pos) = config.position {
                if let Some((x_str, y_str)) = pos.split_once(',') {
                    if let (Ok(x), Ok(y)) = (x_str.parse::<i32>(), y_str.parse::<i32>()) {
                        current_location = (x, y).into();
                    }
                }
            }

            output.change_current_state(
                current_mode,
                Some(current_transform),
                Some(current_scale),
                Some(current_location),
            );
            self.space.map_output(&output, current_location);
        }
    }

    pub fn sync_space_from_globals(&mut self) {
        let Some(g) = self.globals() else {
            return;
        };
        let updates: Vec<(WindowId, Window, Rect, i32)> = self
            .space
            .elements()
            .filter_map(|window| {
                let marker = window.user_data().get::<WindowIdMarker>()?;
                let client = g.clients.get(&marker.id)?;
                Some((marker.id, window.clone(), client.geo, client.border_width))
            })
            .collect();
        for (window_id, window, geo, bw) in updates {
            self.set_window_target_location(
                window_id,
                window.clone(),
                Point::from((geo.x + bw, geo.y + bw)),
                false,
            );
            if let Some(toplevel) = window.toplevel() {
                let key = window
                    .user_data()
                    .get::<WindowIdMarker>()
                    .map(|m| m.id)
                    .unwrap_or_default();
                let target = (geo.w.max(1), geo.h.max(1));
                let unchanged = self
                    .last_configured_size
                    .get(&key)
                    .is_some_and(|&s| s == target);
                if !unchanged {
                    let size = smithay::utils::Size::<i32, smithay::utils::Logical>::new(
                        target.0, target.1,
                    );
                    toplevel.with_pending_state(|state| {
                        state.size = Some(size);
                    });
                    toplevel.send_pending_configure();
                    self.last_configured_size.insert(key, target);
                }
            }
        }
        self.raise_unmanaged_x11_windows();
    }

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
            toplevel.with_pending_state(|state| {
                state.size = Some(size);
            });
            toplevel.send_pending_configure();
            self.last_configured_size.insert(window_id, target);
        }
        if let Some(g) = self.globals_mut() {
            g.layout_dirty = true;
            g.space_dirty = true;
        }
        self.set_focus(window_id);
        window_id
    }

    pub fn resize_window(&mut self, window: WindowId, rect: Rect) {
        if let Some(element) = self.find_window(window).cloned() {
            let bw = self
                .globals()
                .and_then(|g| g.clients.get(&window).map(|c| c.border_width))
                .unwrap_or(0);
            let remap_immediately = self.interactive_motion_active();
            self.set_window_target_location(
                window,
                element.clone(),
                Point::from((rect.x + bw, rect.y + bw)),
                remap_immediately,
            );
            if let Some(toplevel) = element.toplevel() {
                let target = (rect.w.max(1), rect.h.max(1));
                let size =
                    smithay::utils::Size::<i32, smithay::utils::Logical>::new(target.0, target.1);
                toplevel.with_pending_state(|state| {
                    state.size = Some(size);
                });
                toplevel.send_pending_configure();
                self.last_configured_size.insert(window, target);
            }
        }
    }

    pub fn raise_window(&mut self, window: WindowId) {
        if let Some(element) = self.find_window(window).cloned() {
            self.space.raise_element(&element, true);
            if element.set_activated(true) {
                if let Some(toplevel) = element.toplevel() {
                    toplevel.send_pending_configure();
                }
            }
        }
        self.raise_unmanaged_x11_windows();
    }

    pub fn restack(&mut self, windows: &[WindowId]) {
        for window in windows.iter() {
            if let Some(element) = self.find_window(*window).cloned() {
                self.space.raise_element(&element, false);
            }
        }
        self.raise_unmanaged_x11_windows();
    }

    pub fn set_focus(&mut self, window: WindowId) {
        let serial = SERIAL_COUNTER.next_serial();
        let focus_window = self.find_window(window).cloned();
        let focus = focus_window.clone().map(KeyboardFocusTarget::Window);

        if let Some(old_id) = self.focused_window {
            if old_id != window {
                if let Some(old_window) = self.window_index.get(&old_id).cloned() {
                    if old_window.set_activated(false) {
                        if let Some(toplevel) = old_window.toplevel() {
                            toplevel.send_pending_configure();
                        }
                    }
                }
            }
        }
        if let Some(new_window) = focus_window {
            if new_window.set_activated(true) {
                if let Some(toplevel) = new_window.toplevel() {
                    toplevel.send_pending_configure();
                }
            }
        }
        self.focused_window = Some(window);

        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, focus, serial);
        }
    }

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

    pub fn map_window(&mut self, window: WindowId) {
        if let Some(element) = self.find_window(window).cloned() {
            // Get the location from the space if the element is already mapped,
            // otherwise use the client's stored geometry to avoid animating from (0,0)
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

    pub fn unmap_window(&mut self, window: WindowId) {
        if let Some(element) = self.window_index.get(&window).cloned() {
            self.space.unmap_elem(&element);
        }
        self.window_animations.remove(&window);
        self.last_configured_size.remove(&window);
        if self.focused_window == Some(window) {
            self.focused_window = None;
            let serial = SERIAL_COUNTER.next_serial();
            if let Some(keyboard) = self.seat.get_keyboard() {
                keyboard.set_focus(self, None::<KeyboardFocusTarget>, serial);
            }
        }
        if let Some(g) = self.globals_mut() {
            g.layout_dirty = true;
            g.space_dirty = true;
        }
    }

    pub(super) fn remove_window_tracking(&mut self, window: WindowId) {
        if let Some(element) = self.window_index.get(&window).cloned() {
            self.space.unmap_elem(&element);
        }
        self.window_index.remove(&window);
        self.window_animations.remove(&window);
        self.last_configured_size.remove(&window);
        if self.focused_window == Some(window) {
            self.focused_window = None;
        }
        if let Some(g) = self.globals_mut() {
            g.layout_dirty = true;
            g.space_dirty = true;
        }
    }

    pub(super) fn restore_focus_after_overlay(&mut self) {
        if let Some(g) = self.globals_mut() {
            if let Some(win) = g.selected_win() {
                self.set_focus(win);
            }
        }
    }

    pub fn set_keyboard_layout(
        &mut self,
        layout: &str,
        variant: &str,
        options: Option<&str>,
        model: Option<&str>,
    ) {
        let config = XkbConfig {
            layout,
            variant,
            options: options.map(|s| s.to_string()),
            model: model.unwrap_or(""),
            rules: "evdev",
        };

        let keyboard = self.keyboard.clone();
        if let Err(e) = keyboard.set_xkb_config(self, config) {
            log::error!("failed to apply wayland keyboard layout: {}", e);
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

    pub fn flush(&mut self) {
        self.space.refresh();
        let _ = self.display_handle.flush_clients();
    }

    fn raise_unmanaged_x11_windows(&mut self) {
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
    pub fn overlay_windows_for_render(&self, x_offset: i32) -> Vec<(Window, Point<i32, Physical>)> {
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

    pub fn window_exists(&self, window: WindowId) -> bool {
        self.window_index.contains_key(&window)
    }

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

    pub fn surface_under_pointer(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<i32, Logical>,
    )> {
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
            if let Some(result) =
                window.surface_under(point - loc.to_f64(), WindowSurfaceType::TOPLEVEL)
            {
                return Some((result.0, result.1 + loc));
            }
        }
        None
    }

    pub fn layer_surface_under_pointer(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<i32, Logical>,
    )> {
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

    pub fn keyboard_focus_layer_surface(
        &self,
    ) -> Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface> {
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

    pub(super) fn find_window(&self, window: WindowId) -> Option<&Window> {
        self.window_index.get(&window)
    }

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

        let mut c = WmClient::default();
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

    pub(super) fn window_id_for_toplevel(&self, surface: &ToplevelSurface) -> Option<WindowId> {
        let wl_surface = surface.wl_surface();
        self.window_index
            .values()
            .find(|w| w.wl_surface().as_deref() == Some(wl_surface))
            .and_then(|w| w.user_data().get::<WindowIdMarker>().map(|m| m.id))
    }

    pub(super) fn window_id_for_x11_surface(
        &self,
        surface: &smithay::xwayland::X11Surface,
    ) -> Option<WindowId> {
        self.window_index
            .values()
            .find(|w| w.x11_surface().is_some_and(|x11| x11 == surface))
            .and_then(|w| w.user_data().get::<WindowIdMarker>().map(|m| m.id))
    }

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
}
