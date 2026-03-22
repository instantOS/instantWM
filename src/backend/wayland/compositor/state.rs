use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

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
        idle_inhibit::IdleInhibitManagerState,
        output::OutputManagerState,
        pointer_gestures::PointerGesturesState,
        relative_pointer::RelativePointerManagerState,
        selection::data_device::DataDeviceState,
        session_lock::{LockSurface, SessionLockManagerState},
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{decoration::XdgDecorationState, XdgShellState},
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
    pub pointer_gestures_state: PointerGesturesState,
    pub relative_pointer_manager_state: RelativePointerManagerState,
    pub viewporter_state: ViewporterState,
    pub idle_inhibit_manager_state: IdleInhibitManagerState,
    pub session_lock_manager_state: SessionLockManagerState,
    pub lock_state: SessionLockState,
    pub lock_surfaces: HashMap<String, LockSurface>,
    pub idle_inhibiting_surfaces: HashSet<WlSurface>,
    pub(super) render_node: Option<DrmNode>,

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

    // -- WM state (owned) --
    pub wm: Wm,

    // -- Renderer (shared via Rc<RefCell> for backends that need shared access) --
    pub renderer: Rc<RefCell<GlesRenderer>>,

    // -- Internal state --
    pub(super) next_window_id: u32,
    pub tracked_devices: Vec<smithay::reexports::input::Device>,
    pub(super) last_configured_size: HashMap<WindowId, (i32, i32)>,
    pub(super) window_index: HashMap<WindowId, Window>,
    pub(super) window_animations: HashMap<WindowId, WaylandWindowAnimation>,
    pub(super) foreign_toplevel_handles: HashMap<WindowId, ForeignToplevelHandle>,
    pub pending_screencopies: Vec<PendingScreencopy>,
    pub(super) pending_toplevels: Vec<smithay::wayland::shell::xdg::ToplevelSurface>,
    pub pending_warp: Option<Point<f64, Logical>>,
    pub pointer_location: Point<f64, Logical>,
    pub led_state_tx: Option<std::sync::mpsc::Sender<smithay::input::keyboard::LedState>>,
    pub dnd_icon: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
    pub pending_libinput_events:
        Vec<smithay::backend::input::InputEvent<smithay::backend::libinput::LibinputInputBackend>>,
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

impl WaylandState {
    /// Create a new `WaylandState` and register all Wayland globals.
    pub fn new(
        display: Display<WaylandState>,
        handle: &LoopHandle<'static, WaylandState>,
        wm: Wm,
        renderer: Rc<RefCell<GlesRenderer>>,
    ) -> Self {
        let dh = display.handle();

        handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, data: &mut WaylandState| {
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
        let compositor_state = CompositorState::new::<WaylandState>(&dh);
        let shm_state = ShmState::new::<WaylandState>(&dh, vec![]);
        let xdg_shell_state = XdgShellState::new::<WaylandState>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<WaylandState>(&dh);
        let xdg_activation_state = XdgActivationState::new::<WaylandState>(&dh);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<WaylandState>(&dh);
        let data_device_state = DataDeviceState::new::<WaylandState>(&dh);
        let xwayland_shell_state = XWaylandShellState::new::<WaylandState>(&dh);
        let xwayland_keyboard_grab_state = XWaylandKeyboardGrabState::new::<WaylandState>(&dh);
        let wlr_layer_shell_state = WlrLayerShellState::new::<WaylandState>(&dh);
        let dmabuf_state = DmabufState::new();
        let foreign_toplevel_list_state = ForeignToplevelListState::new::<WaylandState>(&dh);
        let pointer_gestures_state = PointerGesturesState::new::<WaylandState>(&dh);
        let relative_pointer_manager_state = RelativePointerManagerState::new::<WaylandState>(&dh);
        let viewporter_state = ViewporterState::new::<WaylandState>(&dh);
        let idle_inhibit_manager_state = IdleInhibitManagerState::new::<WaylandState>(&dh);
        let session_lock_manager_state =
            SessionLockManagerState::new::<WaylandState, _>(&dh, |_| true);

        // -- Seat (input devices) --
        let cursor_config = wm.g.cfg.cursor.clone();
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
            pointer_gestures_state,
            relative_pointer_manager_state,
            viewporter_state,
            idle_inhibit_manager_state,
            session_lock_manager_state,
            lock_state: SessionLockState::Unlocked,
            lock_surfaces: HashMap::new(),
            idle_inhibiting_surfaces: HashSet::new(),
            render_node: None,
            seat,
            keyboard,
            pointer,
            cursor_config,
            cursor_image_status: smithay::input::pointer::CursorImageStatus::default_named(),
            cursor_icon_override: None,
            xwm: None,
            xdisplay: None,
            wm,
            renderer,
            next_window_id: 1,
            tracked_devices: Vec::new(),
            last_configured_size: HashMap::new(),
            window_index: HashMap::new(),
            window_animations: HashMap::new(),
            foreign_toplevel_handles: HashMap::new(),
            pending_screencopies: Vec::new(),
            pending_toplevels: Vec::new(),
            pending_warp: None,
            pointer_location: Point::from((0.0, 0.0)),
            led_state_tx: None,
            dnd_icon: None,
            pending_libinput_events: Vec::new(),
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
                .create_global_with_default_feedback::<WaylandState>(
                    &self.display_handle,
                    &feedback,
                )
        } else {
            log::info!("dmabuf: no render node available, falling back to zwp_linux_dmabuf_v1 v3");
            self.dmabuf_state
                .create_global::<WaylandState>(&self.display_handle, formats)
        });
    }

    pub fn sync_space_from_globals(&mut self) {
        let dead_windows: Vec<WindowId> = self
            .window_index
            .iter()
            .filter_map(|(&id, w)| if !w.alive() { Some(id) } else { None })
            .collect();

        for win in dead_windows {
            self.remove_window_tracking(win);
            self.wm.g.detach(win);
            self.wm.g.detach_stack(win);
            self.wm.g.clients.remove(&win);
        }

        self.restore_focus_after_overlay();

        let g = &self.wm.g;
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
                .is_some_and(|s| *s == target);
            if !unchanged {
                let size =
                    smithay::utils::Size::<i32, smithay::utils::Logical>::new(target.0, target.1);
                self.send_toplevel_configure(&window, Some(size));
                self.last_configured_size.insert(key, target);
            }
        }
        self.raise_unmanaged_x11_windows();
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

    pub fn is_locked(&self) -> bool {
        matches!(self.lock_state, SessionLockState::Locked(_))
    }

    pub fn flush(&mut self) {
        self.space.refresh();
        let _ = self.display_handle.flush_clients();
    }
}
