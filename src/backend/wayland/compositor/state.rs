use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::NonNull;

use smithay::utils::IsAlive;
use smithay::{
    backend::allocator::Format,
    backend::drm::DrmNode,
    backend::egl::{EGLDevice, EGLDisplay},
    backend::renderer::gles::GlesRenderer,
    desktop::{PopupManager, Space, Window},
    input::{
        keyboard::{KeyboardHandle, XkbConfig},
        pointer::PointerHandle,
        Seat, SeatState,
    },
    reexports::{
        calloop::{generic::Generic, Interest, LoopHandle, Mode, PostAction},
        wayland_server::{Display, DisplayHandle},
    },
    utils::{Logical, Point},
    wayland::{
        compositor::CompositorState,
        dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufState},
        foreign_toplevel_list::{ForeignToplevelHandle, ForeignToplevelListState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{decoration::XdgDecorationState, XdgShellState},
        },
        shm::ShmState,
        xdg_activation::XdgActivationState,
        xwayland_keyboard_grab::XWaylandKeyboardGrabState,
        xwayland_shell::XWaylandShellState,
    },
    xwayland::X11Wm,
};

use crate::globals::Globals;
use crate::types::{Rect, WindowId};
use crate::wm::Wm;

use super::screencopy::PendingScreencopy;
use super::window::WaylandWindowAnimation;

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
    pub xdg_activation_state: XdgActivationState,
    pub seat_state: SeatState<WaylandState>,
    pub output_manager_state: OutputManagerState,
    pub data_device_state: DataDeviceState,
    pub xwayland_shell_state: XWaylandShellState,
    pub xwayland_keyboard_grab_state: XWaylandKeyboardGrabState,
    pub wlr_layer_shell_state: WlrLayerShellState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub foreign_toplevel_list_state: ForeignToplevelListState,
    /// DRM node used for rendering, needed to tag imported dmabufs.
    pub(super) render_node: Option<DrmNode>,
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

    // -- Internal state --
    pub(super) next_window_id: u32,
    wm: Option<NonNull<Wm>>,
    pub tracked_devices: Vec<smithay::reexports::input::Device>,
    pub(super) last_configured_size: HashMap<WindowId, (i32, i32)>,
    /// O(1) window lookup index containing all known windows (mapped and hidden).
    pub(super) window_index: HashMap<WindowId, Window>,
    pub(super) window_animations: HashMap<WindowId, WaylandWindowAnimation>,
    /// Foreign toplevel handles for each window (for taskbar/panel support).
    pub(super) foreign_toplevel_handles: HashMap<WindowId, ForeignToplevelHandle>,

    /// Pending screencopy frames waiting to be fulfilled during the next render.
    pub pending_screencopies: Vec<PendingScreencopy>,

    /// Pending cursor warp requested by the WM (e.g. warp-to-focus keybinding).
    /// The event loop consumes this each tick and synthesises a pointer motion.
    pub pending_warp: Option<Point<f64, Logical>>,

    /// Current pointer location in logical coordinates.
    /// Stored centrally to ensure consistent state across backends.
    pub pointer_location: Point<f64, Logical>,

    /// Channel to notify the DRM backend loop of keyboard LED state changes.
    pub led_state_tx: Option<std::sync::mpsc::Sender<smithay::input::keyboard::LedState>>,

    /// Current drag-and-drop icon surface.
    pub dnd_icon: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowIdMarker {
    pub id: WindowId,
    /// Cached: true when this is an unmanaged X11 overlay (dmenu, popup, etc.).
    pub is_overlay: bool,
}

impl WaylandState {
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
        let xdg_activation_state = XdgActivationState::new::<Self>(&dh);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let xwayland_shell_state = XWaylandShellState::new::<Self>(&dh);
        let xwayland_keyboard_grab_state = XWaylandKeyboardGrabState::new::<Self>(&dh);
        let wlr_layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let dmabuf_state = DmabufState::new();
        let foreign_toplevel_list_state = ForeignToplevelListState::new::<Self>(&dh);

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
            xdg_activation_state,
            seat_state,
            output_manager_state,
            data_device_state,
            xwayland_shell_state,
            xwayland_keyboard_grab_state,
            wlr_layer_shell_state,
            dmabuf_state,
            dmabuf_global: None,
            foreign_toplevel_list_state,
            render_node: None,
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
            foreign_toplevel_handles: HashMap::new(),
            pending_screencopies: Vec::new(),
            pending_warp: None,
            pointer_location: Point::from((0.0, 0.0)),
            led_state_tx: None,
            dnd_icon: None,
        }
    }

    /// Attach the WM to this state.
    pub fn attach_wm(&mut self, wm: &mut Wm) {
        self.wm = Some(NonNull::from(wm));
    }

    /// Execute a closure with access to both WaylandState and Wm.
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

        // Store the render node so we can tag imported dmabufs with it.
        self.render_node = render_node;

        self.dmabuf_global = Some(if let Some(node) = self.render_node {
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

    /// Attach the GLES renderer.
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

    /// Get mutable reference to the renderer.
    pub(super) fn renderer_mut(&mut self) -> Option<&mut GlesRenderer> {
        self.renderer.map(|mut p| unsafe { p.as_mut() })
    }

    #[inline]
    pub(super) fn globals(&self) -> Option<&Globals> {
        self.wm.map(|p: NonNull<Wm>| unsafe { &p.as_ref().g })
    }

    #[inline]
    pub(super) fn globals_mut(&mut self) -> Option<&mut Globals> {
        self.wm
            .map(|mut p: NonNull<Wm>| unsafe { &mut p.as_mut().g })
    }

    /// Sync the Smithay space from the Globals state.
    pub fn sync_space_from_globals(&mut self) {
        let dead_windows: Vec<WindowId> = self
            .window_index
            .iter()
            .filter_map(|(&id, w)| if !w.alive() { Some(id) } else { None })
            .collect();

        for win in dead_windows {
            // Use the method from window.rs
            self.remove_window_tracking(win);
            if let Some(g) = self.globals_mut() {
                g.detach(win);
                g.detach_stack(win);
                g.clients.remove(&win);
            }
        }

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
            // Use the method from window.rs
            let target_point = Point::from((geo.x + bw, geo.y + bw));
            self.set_window_target_location(window_id, window.clone(), target_point, false);
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
                let size =
                    smithay::utils::Size::<i32, smithay::utils::Logical>::new(target.0, target.1);
                // Use the method from window.rs
                self.send_toplevel_configure(&window, Some(size));
                self.last_configured_size.insert(key, target);
            }
        }
        self.raise_unmanaged_x11_windows();
    }

    /// Set the keyboard layout.
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

    /// Flush pending data to clients.
    pub fn flush(&mut self) {
        self.space.refresh();
        let _ = self.display_handle.flush_clients();
    }
}
