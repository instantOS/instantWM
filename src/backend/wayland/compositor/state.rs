use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;

use smithay::reexports::wayland_protocols::ext::session_lock::v1::server::ext_session_lock_v1::ExtSessionLockV1;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::IsAlive;
use smithay::{
    backend::allocator::Format,
    backend::drm::DrmNode,
    backend::egl::{EGLDevice, EGLDisplay},
    backend::renderer::gles::GlesRenderer,
    desktop::{PopupManager, Space, Window},
    input::{
        Seat, SeatState,
        keyboard::{KeyboardHandle, XkbConfig},
        pointer::PointerHandle,
    },
    reexports::{
        calloop::{Interest, LoopHandle, Mode, PostAction, generic::Generic},
        wayland_server::{Display, DisplayHandle},
    },
    utils::{Logical, Point},
    wayland::{
        compositor::CompositorState,
        dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufState},
        foreign_toplevel_list::{ForeignToplevelHandle, ForeignToplevelListState},
        idle_inhibit::IdleInhibitManagerState,
        image_capture_source::{ImageCaptureSourceState, OutputCaptureSourceState},
        image_copy_capture::{ImageCopyCaptureState, Session as ImageCopySession},
        output::OutputManagerState,
        pointer_gestures::PointerGesturesState,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        selection::{
            data_device::DataDeviceState,
            ext_data_control::DataControlState as ExtDataControlState,
            wlr_data_control::DataControlState as WlrDataControlState,
        },
        session_lock::{LockSurface, SessionLockManagerState},
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{XdgShellState, decoration::XdgDecorationState},
        },
        shm::ShmState,
        viewporter::ViewporterState,
        xdg_activation::XdgActivationState,
        xwayland_keyboard_grab::XWaylandKeyboardGrabState,
        xwayland_shell::XWaylandShellState,
    },
    xwayland::X11Wm,
};

use crate::config::config_toml::CursorConfig;
use crate::config::config_toml::VrrMode;
use crate::globals::Globals;
use crate::types::{Rect, WindowId};
use crate::wm::Wm;

use super::image_capture::PendingImageCapture;
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
    pub presentation_state: PresentationState,
    pub data_device_state: DataDeviceState,
    pub ext_data_control_state: ExtDataControlState,
    pub wlr_data_control_state: WlrDataControlState,
    pub xwayland_shell_state: XWaylandShellState,
    pub xwayland_keyboard_grab_state: XWaylandKeyboardGrabState,
    pub wlr_layer_shell_state: WlrLayerShellState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub foreign_toplevel_list_state: ForeignToplevelListState,
    pub image_capture_source_state: ImageCaptureSourceState,
    pub output_capture_source_state: OutputCaptureSourceState,
    pub image_copy_capture_state: ImageCopyCaptureState,
    pub pointer_gestures_state: PointerGesturesState,
    pub relative_pointer_manager_state: RelativePointerManagerState,
    pub viewporter_state: ViewporterState,
    pub idle_inhibit_manager_state: IdleInhibitManagerState,
    pub session_lock_manager_state: SessionLockManagerState,
    /// Current session lock state.
    pub lock_state: SessionLockState,
    /// Lock surfaces per output (keyed by output name).
    pub lock_surfaces: HashMap<String, LockSurface>,
    /// Surfaces that have active idle inhibitors.
    pub idle_inhibiting_surfaces: HashSet<WlSurface>,
    /// DRM node used for rendering, needed to tag imported dmabufs.
    pub(super) render_node: Option<DrmNode>,
    renderer: Option<NonNull<GlesRenderer>>,

    // -- Input --
    pub seat: Seat<WaylandState>,
    pub keyboard: KeyboardHandle<WaylandState>,
    pub pointer: PointerHandle<WaylandState>,
    pub cursor_config: CursorConfig,
    pub cursor_image_status: smithay::input::pointer::CursorImageStatus,
    pub cursor_icon_override: Option<smithay::input::pointer::CursorIcon>,

    // -- XWayland --
    pub xwm: Option<X11Wm>,
    pub xdisplay: Option<u32>,

    // -- Internal state --
    pub(super) next_window_id: u32,
    /// Back-reference to the main WM state.
    ///
    /// This is a raw pointer because `Wm` owns the `Backend`, which in turn
    /// wants to reference `WaylandState`. Since `WaylandState` is owned by
    /// the event loop, a standard `Rc/RefCell` cycle would be difficult
    /// to manage and performantly access from Smithay's handlers.
    wm: Option<NonNull<Wm>>,
    pub(super) last_configured_size: HashMap<WindowId, (i32, i32)>,
    pub(super) active_resizes: HashSet<WindowId>,
    /// O(1) window lookup index containing all known windows (mapped and hidden).
    pub(super) window_index: HashMap<WindowId, Window>,
    pub(super) window_animations: HashMap<WindowId, WaylandWindowAnimation>,
    /// Foreign toplevel handles for each window (for taskbar/panel support).
    pub(super) foreign_toplevel_handles: HashMap<WindowId, ForeignToplevelHandle>,

    /// Pending cursor warp requested by the WM (e.g. warp-to-focus keybinding).
    /// The event loop consumes this each tick and synthesises a pointer motion.
    pub pending_warp: Option<Point<f64, Logical>>,
    /// Backend-local runtime state that is not part of protocol or desktop state.
    pub runtime: WaylandRuntimeState,
}

/// Tracks the current session lock state.
#[derive(Debug, Default)]
pub enum SessionLockState {
    #[default]
    Unlocked,
    Locked(ExtSessionLockV1),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowIdMarker {
    pub id: WindowId,
    /// Cached: true when this is an unmanaged X11 overlay (dmenu, popup, etc.).
    pub is_overlay: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaylandOutputMetadata {
    pub vrr_support: crate::backend::BackendVrrSupport,
    pub vrr_mode: VrrMode,
    pub vrr_enabled: bool,
}

pub struct WaylandRuntimeState {
    pub tracked_devices: Vec<smithay::reexports::input::Device>,
    pub pending_screencopies: Vec<PendingScreencopy>,
    pub pending_image_captures: Vec<PendingImageCapture>,
    pub image_copy_sessions: Vec<ImageCopySession>,
    pub render_dirty: bool,
    pub render_ping: Option<smithay::reexports::calloop::ping::Ping>,
    pub output_metadata: HashMap<String, WaylandOutputMetadata>,
    pub pending_toplevels: Vec<smithay::wayland::shell::xdg::ToplevelSurface>,
    pub pointer_location: Point<f64, Logical>,
    pub led_state_tx: Option<std::sync::mpsc::Sender<smithay::input::keyboard::LedState>>,
    pub dnd_icon: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
    pub winit_window_size: smithay::utils::Size<i32, smithay::utils::Physical>,
    pub pending_winit_resize: Option<(i32, i32)>,
    pub winit_close_requested: bool,
}

impl Default for WaylandRuntimeState {
    fn default() -> Self {
        Self {
            tracked_devices: Vec::new(),
            pending_screencopies: Vec::new(),
            pending_image_captures: Vec::new(),
            image_copy_sessions: Vec::new(),
            render_dirty: false,
            render_ping: None,
            output_metadata: HashMap::new(),
            pending_toplevels: Vec::new(),
            pointer_location: Point::from((0.0, 0.0)),
            led_state_tx: None,
            dnd_icon: None,
            winit_window_size: smithay::utils::Size::from((0, 0)),
            pending_winit_resize: None,
            winit_close_requested: false,
        }
    }
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
        let presentation_state = PresentationState::new::<Self>(&dh, libc::CLOCK_MONOTONIC as u32);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let ext_data_control_state = ExtDataControlState::new::<Self, _>(&dh, None, |_| true);
        let wlr_data_control_state = WlrDataControlState::new::<Self, _>(&dh, None, |_| true);
        let xwayland_shell_state = XWaylandShellState::new::<Self>(&dh);
        let xwayland_keyboard_grab_state = XWaylandKeyboardGrabState::new::<Self>(&dh);
        let wlr_layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let dmabuf_state = DmabufState::new();
        let foreign_toplevel_list_state = ForeignToplevelListState::new::<Self>(&dh);
        let image_capture_source_state = ImageCaptureSourceState::new();
        let output_capture_source_state = OutputCaptureSourceState::new::<Self>(&dh);
        let image_copy_capture_state = ImageCopyCaptureState::new::<Self>(&dh);
        let pointer_gestures_state = PointerGesturesState::new::<Self>(&dh);
        let relative_pointer_manager_state = RelativePointerManagerState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let idle_inhibit_manager_state = IdleInhibitManagerState::new::<Self>(&dh);
        let session_lock_manager_state = SessionLockManagerState::new::<Self, _>(&dh, |_| true);

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
            presentation_state,
            data_device_state,
            ext_data_control_state,
            wlr_data_control_state,
            xwayland_shell_state,
            xwayland_keyboard_grab_state,
            wlr_layer_shell_state,
            dmabuf_state,
            dmabuf_global: None,
            foreign_toplevel_list_state,
            image_capture_source_state,
            output_capture_source_state,
            image_copy_capture_state,
            pointer_gestures_state,
            relative_pointer_manager_state,
            viewporter_state,
            idle_inhibit_manager_state,
            session_lock_manager_state,
            lock_state: SessionLockState::Unlocked,
            lock_surfaces: HashMap::new(),
            idle_inhibiting_surfaces: HashSet::new(),
            render_node: None,
            renderer: None,
            seat,
            keyboard,
            pointer,
            cursor_config: CursorConfig::default(),
            cursor_image_status: smithay::input::pointer::CursorImageStatus::default_named(),
            cursor_icon_override: None,
            xwm: None,
            xdisplay: None,
            next_window_id: 1,
            wm: None,
            last_configured_size: HashMap::new(),
            active_resizes: HashSet::new(),
            window_index: HashMap::new(),
            window_animations: HashMap::new(),
            foreign_toplevel_handles: HashMap::new(),
            pending_warp: None,
            runtime: WaylandRuntimeState::default(),
        }
    }

    pub fn init_dmabuf_global(&mut self, formats: Vec<Format>, egl_display: Option<&EGLDisplay>) {
        if self.dmabuf_global.is_some() {
            return;
        }

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
    #[allow(unexpected_cfgs)]
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
            use smithay::backend::renderer::ImportEgl;
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

    /// Attach the WM to this state.
    pub fn attach_wm(&mut self, wm: &mut Wm) {
        self.cursor_config = wm.g.cfg.cursor.clone();
        self.wm = Some(NonNull::from(wm));
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

    /// Execute a closure with a mutable reference to the WM and WaylandState.
    /// This is a specialized helper to avoid double-borrowing when we need
    /// to pass `&mut WaylandState` to a function that also needs `&mut Wm`.
    pub fn with_wm_mut_unified<T>(
        &mut self,
        f: impl FnOnce(&mut Wm, &mut WaylandState) -> T,
    ) -> Option<T> {
        self.wm.map(|mut p| {
            let wm = unsafe { p.as_mut() };
            f(wm, self)
        })
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

        // Only recover focus when the seat focus is actually missing or dead.
        // A plain space sync must not steal focus away from a live overlay
        // surface such as fuzzel/rofi, or from any other valid keyboard target.
        let seat_focus_needs_recovery = self
            .seat
            .get_keyboard()
            .and_then(|k| k.current_focus())
            .is_none_or(|focus| !focus.alive());
        if seat_focus_needs_recovery {
            self.restore_focus_after_overlay();
        }

        let Some(g) = self.globals() else {
            return;
        };
        let updates: Vec<(WindowId, Rect)> = self
            .space
            .elements()
            .filter_map(|window| {
                let marker = window.user_data().get::<WindowIdMarker>()?;
                let client = g.clients.get(&marker.id)?;
                Some((marker.id, client.geo))
            })
            .collect();
        for (window_id, geo) in updates {
            self.set_window_target_rect(
                window_id,
                geo,
                super::window::animations::WindowMoveMode::Normal,
            );
        }
        self.raise_unmanaged_x11_windows();
    }

    #[inline]
    pub fn request_render(&mut self) {
        self.runtime.render_dirty = true;
        if let Some(render_ping) = &self.runtime.render_ping {
            render_ping.ping();
        }
    }

    #[inline]
    pub fn request_bar_redraw(&mut self) {
        let _ = self.with_wm_mut_unified(|wm, _state| {
            wm.bar.mark_dirty();
        });
        self.request_render();
    }

    #[inline]
    pub fn take_render_dirty(&mut self) -> bool {
        std::mem::take(&mut self.runtime.render_dirty)
    }

    pub fn set_output_vrr_support(
        &mut self,
        output_name: &str,
        support: crate::backend::BackendVrrSupport,
    ) {
        let entry = self
            .runtime
            .output_metadata
            .entry(output_name.to_string())
            .or_insert(WaylandOutputMetadata {
                vrr_support: support,
                vrr_mode: VrrMode::Auto,
                vrr_enabled: false,
            });
        entry.vrr_support = support;
    }

    pub fn set_output_vrr_mode(&mut self, output_name: &str, mode: VrrMode) {
        let entry = self
            .runtime
            .output_metadata
            .entry(output_name.to_string())
            .or_insert(WaylandOutputMetadata {
                vrr_support: crate::backend::BackendVrrSupport::Unsupported,
                vrr_mode: mode,
                vrr_enabled: false,
            });
        entry.vrr_mode = mode;
    }

    pub fn set_output_vrr_enabled(&mut self, output_name: &str, enabled: bool) {
        let entry = self
            .runtime
            .output_metadata
            .entry(output_name.to_string())
            .or_insert(WaylandOutputMetadata {
                vrr_support: crate::backend::BackendVrrSupport::Unsupported,
                vrr_mode: VrrMode::Auto,
                vrr_enabled: enabled,
            });
        entry.vrr_enabled = enabled;
    }

    pub fn output_vrr_metadata(&self, output_name: &str) -> Option<&WaylandOutputMetadata> {
        self.runtime.output_metadata.get(output_name)
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

    /// Returns `true` if the session is currently locked.
    pub fn is_locked(&self) -> bool {
        matches!(self.lock_state, SessionLockState::Locked(_))
    }

    /// Flush pending data to clients.
    pub fn flush(&mut self) {
        self.space.refresh();
        let _ = self.display_handle.flush_clients();
    }
}
