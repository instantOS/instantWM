use crate::contexts::WmCtx;
use std::collections::{HashMap, HashSet};

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
        keyboard::{KeyboardHandle, Keycode, XkbConfig},
        pointer::PointerHandle,
        touch::TouchHandle,
    },
    reexports::{
        calloop::{Interest, LoopHandle, Mode, PostAction, generic::Generic},
        wayland_server::{Display, DisplayHandle},
    },
    utils::{Logical, Point},
    wayland::{
        alpha_modifier::AlphaModifierState,
        commit_timing::CommitTimingManagerState,
        compositor::CompositorState,
        content_type::ContentTypeState,
        cursor_shape::CursorShapeManagerState,
        dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufState},
        drm_syncobj::DrmSyncobjState,
        fifo::FifoManagerState,
        fixes::FixesState,
        foreign_toplevel_list::{ForeignToplevelHandle, ForeignToplevelListState},
        fractional_scale::FractionalScaleManagerState,
        idle_inhibit::IdleInhibitManagerState,
        idle_notify::IdleNotifierState,
        image_capture_source::{ImageCaptureSourceState, OutputCaptureSourceState},
        image_copy_capture::{ImageCopyCaptureState, Session as ImageCopySession},
        input_method::InputMethodManagerState,
        keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState,
        output::OutputManagerState,
        pointer_constraints::PointerConstraintsState,
        pointer_gestures::PointerGesturesState,
        pointer_warp::PointerWarpManager,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        selection::{
            data_device::DataDeviceState,
            ext_data_control::DataControlState as ExtDataControlState,
            primary_selection::PrimarySelectionState,
            wlr_data_control::DataControlState as WlrDataControlState,
        },
        session_lock::{LockSurface, SessionLockManagerState},
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{XdgShellState, decoration::XdgDecorationState, dialog::XdgDialogState},
        },
        shm::ShmState,
        single_pixel_buffer::SinglePixelBufferState,
        tablet_manager::TabletManagerState,
        text_input::TextInputManagerState,
        viewporter::ViewporterState,
        virtual_keyboard::VirtualKeyboardManagerState,
        xdg_activation::XdgActivationState,
        xdg_foreign::XdgForeignState,
        xwayland_keyboard_grab::XWaylandKeyboardGrabState,
        xwayland_shell::XWaylandShellState,
    },
    xwayland::X11Wm,
};

use super::protocols::ext_workspace::ExtWorkspaceManagerState;
use super::protocols::output_management::OutputManagementState;
use super::protocols::output_power::OutputPowerState;
use crate::config::config_toml::CursorConfig;
use crate::types::{Rect, WindowId};
use crate::wm::WaylandWm as Wm;

use super::image_capture::PendingImageCapture;
use super::screencopy::PendingScreencopy;

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
/// `CoreState` for shared WM state (tags, clients, config, etc.).
pub struct WaylandState {
    pub native: WaylandNativeState,
    pub(crate) graphics: Option<super::graphics::Graphics>,
    /// Sole policy owner. A context borrows the root and reborrows this field
    /// only while computing policy. Smithay seat callbacks require the root,
    /// so contexts must not retain an independent core borrow across effects.
    pub(crate) wm: Wm,
}

/// Protocol and scene data, separate from the graphics owner so frames borrow
/// their renderer and scene exclusively as disjoint fields.
/// Model-dependent native methods take an explicit `&CoreState` so callers can
/// borrow the model and scene independently. Do not replace those parameters
/// with a WM back-reference or `RefCell`: Smithay callbacks need the complete
/// root, and must remain statically excluded while these field borrows live.
pub struct WaylandNativeState {
    // -- Wayland infrastructure --
    pub display_handle: DisplayHandle,

    // -- Desktop abstractions --
    pub space: Space<Window>,
    pub popups: PopupManager,

    // -- Protocol states --
    pub alpha_modifier_state: AlphaModifierState,
    pub compositor_state: CompositorState,
    pub content_type_state: ContentTypeState,
    pub commit_timing_manager_state: CommitTimingManagerState,
    pub cursor_shape_manager_state: CursorShapeManagerState,
    pub fixes_state: FixesState,
    pub fractional_scale_manager_state: FractionalScaleManagerState,
    pub shm_state: ShmState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub xdg_dialog_state: XdgDialogState,
    pub xdg_activation_state: XdgActivationState,
    pub xdg_foreign_state: XdgForeignState,
    pub seat_state: SeatState<WaylandState>,
    pub output_manager_state: OutputManagerState,
    pub presentation_state: PresentationState,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub ext_data_control_state: ExtDataControlState,
    pub wlr_data_control_state: WlrDataControlState,
    pub xwayland_shell_state: XWaylandShellState,
    pub xwayland_keyboard_grab_state: XWaylandKeyboardGrabState,
    pub wlr_layer_shell_state: WlrLayerShellState,
    pub loop_handle: LoopHandle<'static, WaylandState>,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub drm_syncobj_state: Option<DrmSyncobjState>,
    pub fifo_manager_state: FifoManagerState,
    /// Live surfaces that have requested a FIFO barrier. Keep each surface
    /// until destruction because a blocked commit can install its next barrier
    /// after the current one is signaled, without another pre-commit callback.
    pub fifo_constraint_surfaces: HashSet<WlSurface>,
    /// Surfaces with commit-timing barriers that have not all become eligible.
    pub commit_timing_surfaces: HashSet<WlSurface>,
    pub foreign_toplevel_list_state: ForeignToplevelListState,
    pub image_capture_source_state: ImageCaptureSourceState,
    pub output_capture_source_state: OutputCaptureSourceState,
    pub image_copy_capture_state: ImageCopyCaptureState,
    pub pointer_gestures_state: PointerGesturesState,
    pub pointer_constraints_state: PointerConstraintsState,
    pub pointer_warp_manager: PointerWarpManager,
    pub relative_pointer_manager_state: RelativePointerManagerState,
    pub single_pixel_buffer_state: SinglePixelBufferState,
    pub tablet_manager_state: TabletManagerState,
    pub viewporter_state: ViewporterState,
    pub virtual_keyboard_manager_state: VirtualKeyboardManagerState,
    pub text_input_manager_state: TextInputManagerState,
    pub input_method_manager_state: InputMethodManagerState,
    pub keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState,
    pub idle_inhibit_manager_state: IdleInhibitManagerState,
    pub idle_notify_manager_state: IdleNotifierState<WaylandState>,
    pub session_lock_manager_state: SessionLockManagerState,
    pub ext_workspace_state: ExtWorkspaceManagerState,
    pub output_management_state: OutputManagementState,
    pub output_power_state: OutputPowerState,
    pub foreign_toplevel_management_state:
        crate::backend::wayland::compositor::protocols::foreign_toplevel::ForeignToplevelManagementState,
    /// Current session lock state.
    pub lock_state: SessionLockState,
    /// Lock surfaces per output (keyed by output name).
    pub lock_surfaces: HashMap<String, LockSurface>,
    /// Surfaces that have active idle inhibitors.
    pub idle_inhibiting_surfaces: HashSet<WlSurface>,
    /// DRM node used for rendering, needed to tag imported dmabufs.
    pub(super) render_node: Option<DrmNode>,

    // -- Input --
    pub seat: Seat<WaylandState>,
    pub keyboard: KeyboardHandle<WaylandState>,
    pub pointer: PointerHandle<WaylandState>,
    pub touch: TouchHandle<WaylandState>,
    pub cursor_config: CursorConfig,
    pub cursor_image_status: smithay::input::pointer::CursorImageStatus,
    pub cursor_icon_override: Option<smithay::input::pointer::CursorIcon>,

    // -- XWayland --
    pub xwm: Option<X11Wm>,
    pub xdisplay: Option<u32>,

    // -- Internal state --
    pub(super) next_window_id: u32,
    pending_commit_clients: Vec<smithay::reexports::wayland_server::Client>,
    /// Desired, dispatched, and acknowledged geometry for each client.
    pub(super) geometry_sync:
        HashMap<WindowId, super::window::geometry_sync::WindowGeometrySync>,
    /// The border width the window was last visually placed under. Model
    /// `border_width` flips before transitions, so animation/runtime code
    /// reads this record to start from the width the window actually showed.
    pub(super) placed_border: HashMap<WindowId, i32>,
    pub(super) native_size_hints: HashMap<WindowId, crate::types::SizeHints>,
    pub(super) active_resize: Option<WindowId>,
    /// O(1) window lookup index containing all known windows (mapped and hidden).
    pub(super) window_index: HashMap<WindowId, Window>,
    pub(super) window_animations:
        HashMap<WindowId, super::window::animations::WaylandWindowAnimation>,
    pub(super) layout_preview_animation: crate::animation::LayoutPreviewAnimation,
    pub(super) layout_preview_style: crate::types::InteractionOutlineStyle,
    pub(super) layout_preview_target: Option<WindowId>,
    /// Foreign toplevel handles for each window (for taskbar/panel support).
    pub(super) foreign_toplevel_handles: HashMap<WindowId, ForeignToplevelHandle>,

    /// Pending cursor warp requested by the WM (e.g. warp-to-focus keybinding).
    /// The event loop consumes this each tick and synthesises a pointer motion.
    pub pending_warp: Option<Point<f64, Logical>>,
    /// Deferred cursor position hint from a locked pointer constraint.
    /// Warped to when the pointer constraint is lifted/unlocked.
    pub cursor_position_hint: Option<(WlSurface, Point<f64, Logical>)>,
    /// Backend-local runtime state that is not part of protocol or desktop state.
    pub runtime: WaylandRuntimeState,
    /// Queue of commands to be processed by the core WM.
    pub(crate) command_queue: std::cell::RefCell<Vec<super::super::commands::WmCommand>>,
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
pub struct PendingLaunchContextMarker {
    pub context: crate::client::LaunchContext,
}

pub use super::output::WaylandOutputMetadata;
pub use super::render::PendingRenderTargets;

pub struct WaylandRuntimeState {
    pub tracked_devices: Vec<smithay::reexports::input::Device>,
    pub pending_screencopies: Vec<PendingScreencopy>,
    pub pending_image_captures: Vec<PendingImageCapture>,
    pub image_copy_sessions: Vec<ImageCopySession>,
    pub space_sync_pending: bool,
    pub render_targets: PendingRenderTargets,
    pub frame_callback_targets: PendingRenderTargets,
    pub render_ping: Option<smithay::reexports::calloop::ping::Ping>,
    pub output_metadata: HashMap<String, WaylandOutputMetadata>,
    pub pending_toplevels: Vec<smithay::wayland::shell::xdg::ToplevelSurface>,
    pub(crate) pending_systray_menu: crate::systray::status_notifier::NativeMenuRequestSlot,
    pub(crate) active_systray_menu: Option<crate::systray::status_notifier::ActiveNativeMenu>,
    pub pointer_location: Point<f64, Logical>,
    /// Touch slot currently captured by the compositor-rendered bar.
    ///
    /// The built-in bar is not a Wayland surface, so a touch sequence that
    /// starts there must remain compositor-owned until its matching up/cancel.
    /// Other slots continue through the native `wl_touch` path.
    pub(crate) wm_gesture_touch_slot: Option<smithay::backend::input::TouchSlot>,
    /// Touch slot emulating a pointer for a client without a `wl_touch` binding.
    pub(crate) pointer_touch_slot: Option<smithay::backend::input::TouchSlot>,
    pub cursor_hidden_by_touch: bool,
    /// Whether the current compositor-rendered cursor is animated
    /// (multiple xcursor frames).  Updated each DRM event-loop tick so
    /// the animation timer can keep animated cursors alive at idle.
    pub cursor_is_animated: bool,
    pub led_state_tx: Option<std::sync::mpsc::Sender<smithay::input::keyboard::LedState>>,
    pub dnd_icon: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
    pub winit_window_size: smithay::utils::Size<i32, smithay::utils::Physical>,
    pub pending_winit_resize: Option<crate::types::Size>,
    pub winit_close_requested: bool,
    pub output_transactions: crate::backend::output::OutputTransactionQueue,
    pub lid_output_policy: crate::backend::output::LidOutputPolicy,
    pub lid_policy_dirty: bool,
    pub lid_switches: HashMap<String, bool>,
    pub output_power: crate::backend::output::OutputPowerQueue,
    /// Authoritative physical power mode for outputs whose active backend
    /// supports DPMS. Absence means the output cannot be power-managed.
    pub output_power_modes: HashMap<String, crate::backend::output::OutputPowerMode>,
    /// Outputs whose logical position is anchored by persistent monitor config.
    pub configured_output_positions: HashSet<String>,
    /// Position ownership recorded when output transactions apply. Absent
    /// outputs are automatically placed.
    pub output_position_sources: HashMap<String, crate::backend::output::OutputPositionSource>,
    /// Desired mirror declarations, rebuilt from monitor config whenever
    /// output configs are applied.
    pub mirror_of: crate::output_mirror::MirrorMap,
    /// Mirror head -> source pairs currently presenting. Established only by
    /// applied output snapshots, since `mirror_of` may describe a policy whose
    /// transaction is still pending. Realized mirrors are absent from the
    /// space and advertise no `wl_output`; the renderer projects their
    /// source's scene onto them.
    pub realized_mirrors: HashMap<String, String>,
    /// Mirrors the last policy projection pinned. The next projection treats
    /// heads that left this set as re-entering the desktop.
    pub projected_mirrors: HashSet<String>,
    pub intercepted_key_releases: HashSet<Keycode>,
    pub(crate) shortcut_recovery:
        crate::backend::wayland::input::keyboard::recovery::ShortcutRecoveryState,
    pub session: Option<smithay::backend::session::libseat::LibSeatSession>,
}

impl Default for WaylandRuntimeState {
    fn default() -> Self {
        Self {
            tracked_devices: Vec::new(),
            pending_screencopies: Vec::new(),
            pending_image_captures: Vec::new(),
            image_copy_sessions: Vec::new(),
            space_sync_pending: true,
            render_targets: PendingRenderTargets::None,
            frame_callback_targets: PendingRenderTargets::None,
            render_ping: None,
            output_metadata: HashMap::new(),
            pending_toplevels: Vec::new(),
            pending_systray_menu: std::sync::Arc::new(std::sync::Mutex::new(None)),
            active_systray_menu: None,
            pointer_location: Point::from((0.0, 0.0)),
            wm_gesture_touch_slot: None,
            pointer_touch_slot: None,
            cursor_hidden_by_touch: false,
            cursor_is_animated: false,
            led_state_tx: None,
            dnd_icon: None,
            winit_window_size: smithay::utils::Size::from((0, 0)),
            pending_winit_resize: None,
            winit_close_requested: false,
            output_transactions: crate::backend::output::OutputTransactionQueue::default(),
            lid_output_policy: Default::default(),
            lid_policy_dirty: false,
            lid_switches: HashMap::new(),
            output_power: crate::backend::output::OutputPowerQueue::default(),
            output_power_modes: HashMap::new(),
            configured_output_positions: HashSet::new(),
            output_position_sources: HashMap::new(),
            mirror_of: Default::default(),
            realized_mirrors: HashMap::new(),
            projected_mirrors: HashSet::new(),
            intercepted_key_releases: HashSet::new(),
            shortcut_recovery: Default::default(),
            session: None,
        }
    }
}

pub(crate) const TOUCH_POINTER_BUTTON_CODE: u32 = 0x110;

impl WaylandState {
    /// Release a pointer button synthesized from a touch sequence, if active.
    pub(crate) fn cancel_touch_pointer_emulation(
        &mut self,
        time: smithay::backend::input::InputTime,
    ) {
        if self.native.runtime.pointer_touch_slot.take().is_none() {
            return;
        }
        let pointer = self.native.pointer.clone();
        pointer.button(
            self,
            &smithay::input::pointer::ButtonEvent {
                button: TOUCH_POINTER_BUTTON_CODE,
                state: smithay::backend::input::ButtonState::Released,
                serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
    }

    pub(crate) fn take_expected_systray_menu_toplevel(
        &mut self,
        client_pid: Option<u32>,
    ) -> Option<crate::systray::status_notifier::NativeMenuRequest> {
        const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(2);
        let mut pending = self.native.runtime.pending_systray_menu.lock().ok()?;
        let request = pending.as_ref()?;
        if request.created.elapsed() > MAX_AGE {
            pending.take();
            return None;
        }
        if !request.matches_client_pid(client_pid) {
            return None;
        }
        pending.take()
    }

    pub(crate) fn active_systray_menu(
        &self,
    ) -> Option<&crate::systray::status_notifier::ActiveNativeMenu> {
        self.native.runtime.active_systray_menu.as_ref()
    }

    /// Whether the active or not-yet-mapped native menu belongs to an item.
    /// Adapters use this before dismissing the menu to implement click-toggle
    /// without letting native compositor state leak into shared mouse policy.
    pub(crate) fn native_systray_menu_matches(&self, service: &str, path: &str) -> bool {
        self.native
            .runtime
            .active_systray_menu
            .as_ref()
            .is_some_and(|menu| {
                menu.service == service && menu.path == path && !menu.close_requested
            })
            || self
                .native
                .runtime
                .pending_systray_menu
                .lock()
                .is_ok_and(|request| {
                    request
                        .as_ref()
                        .is_some_and(|menu| menu.service == service && menu.path == path)
                })
    }

    /// Dismiss a native menu whether its toplevel is active or still pending.
    pub(crate) fn dismiss_native_systray_menu(&mut self) -> bool {
        let dismissed_pending = self
            .native
            .runtime
            .pending_systray_menu
            .lock()
            .is_ok_and(|mut pending| pending.take().is_some());
        let Some(active) = self.native.runtime.active_systray_menu.as_mut() else {
            return dismissed_pending;
        };
        let should_send_close = !active.close_requested;
        active.close_requested = true;
        if should_send_close {
            let win = active.win;
            self.close_window(win);
        }
        true
    }

    pub(crate) fn clear_active_systray_menu(&mut self, win: crate::types::WindowId) {
        if self
            .native
            .runtime
            .active_systray_menu
            .as_ref()
            .is_some_and(|active| active.win == win)
        {
            self.native.runtime.active_systray_menu = None;
        }
    }

    /// Create a new `WaylandState` and register all Wayland globals.
    pub fn new(
        display: Display<WaylandState>,
        handle: &LoopHandle<'static, WaylandState>,
        wm: Wm,
    ) -> Self {
        let dh = display.handle();
        let cursor_config = wm.core.state.config.cursor.clone();

        // Insert the Wayland display as a calloop source so that protocol
        // messages from connected clients are dispatched on each loop tick.
        handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, data| {
                    if let Err(err) = unsafe { display.get_mut().dispatch_clients(data) } {
                        log::warn!("wayland dispatch error: {}", err);
                    }
                    Ok(PostAction::Continue)
                },
            )
            .expect("Failed to insert Wayland display source");

        // -- Protocol globals --
        let alpha_modifier_state = AlphaModifierState::new::<Self>(&dh);
        let compositor_state = CompositorState::new_v6::<Self>(&dh);
        let content_type_state = ContentTypeState::new::<Self>(&dh);
        let commit_timing_manager_state = CommitTimingManagerState::new::<Self>(&dh);
        let cursor_shape_manager_state = CursorShapeManagerState::new::<Self>(&dh);
        let fixes_state = FixesState::new::<Self>(&dh);
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let xdg_dialog_state = XdgDialogState::new::<Self>(&dh);
        let xdg_activation_state = XdgActivationState::new::<Self>(&dh);
        let xdg_foreign_state = XdgForeignState::new::<Self>(&dh);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let presentation_state = PresentationState::new::<Self>(&dh, libc::CLOCK_MONOTONIC as u32);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let ext_data_control_state = ExtDataControlState::new::<Self, _>(&dh, None, |_| true);
        let wlr_data_control_state = WlrDataControlState::new::<Self, _>(&dh, None, |_| true);
        let xwayland_shell_state = XWaylandShellState::new::<Self>(&dh);
        let xwayland_keyboard_grab_state = XWaylandKeyboardGrabState::new::<Self>(&dh);
        let wlr_layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let dmabuf_state = DmabufState::new();
        let fifo_manager_state = FifoManagerState::new::<Self>(&dh);
        let foreign_toplevel_list_state = ForeignToplevelListState::new::<Self>(&dh);
        let image_capture_source_state = ImageCaptureSourceState::new();
        let output_capture_source_state = OutputCaptureSourceState::new::<Self>(&dh);
        let image_copy_capture_state = ImageCopyCaptureState::new::<Self>(&dh);
        let pointer_gestures_state = PointerGesturesState::new::<Self>(&dh);
        let pointer_constraints_state = PointerConstraintsState::new::<Self>(&dh);
        let pointer_warp_manager = PointerWarpManager::new::<Self>(&dh);
        let relative_pointer_manager_state = RelativePointerManagerState::new::<Self>(&dh);
        let single_pixel_buffer_state = SinglePixelBufferState::new::<Self>(&dh);
        let tablet_manager_state = TabletManagerState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let virtual_keyboard_manager_state =
            VirtualKeyboardManagerState::new::<Self, _>(&dh, |_| true);
        let text_input_manager_state = TextInputManagerState::new::<Self>(&dh);
        let input_method_manager_state = InputMethodManagerState::new::<Self, _>(&dh, |_| true);
        let keyboard_shortcuts_inhibit_state = KeyboardShortcutsInhibitState::new::<Self>(&dh);
        let idle_inhibit_manager_state = IdleInhibitManagerState::new::<Self>(&dh);
        let idle_notify_manager_state = IdleNotifierState::new(&dh, handle.clone());
        let session_lock_manager_state = SessionLockManagerState::new::<Self, _>(&dh, |_| true);
        let ext_workspace_state = ExtWorkspaceManagerState::new(&dh);
        let output_management_state = OutputManagementState::new::<Self>(&dh);
        let output_power_state = OutputPowerState::new::<Self>(&dh);
        let foreign_toplevel_management_state =
            crate::backend::wayland::compositor::protocols::foreign_toplevel::ForeignToplevelManagementState::new::<Self>(&dh);

        // -- Seat (input devices) --
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "seat-0");
        let keyboard = seat
            .add_keyboard(XkbConfig::default(), 400, 25)
            .expect("Failed to add keyboard to seat");
        let pointer = seat.add_pointer();
        let touch = seat.add_touch();

        Self {
            wm,
            graphics: None,
            native: WaylandNativeState {
                display_handle: dh,
                space: Space::default(),
                popups: PopupManager::default(),
                alpha_modifier_state,
                compositor_state,
                content_type_state,
                commit_timing_manager_state,
                cursor_shape_manager_state,
                fixes_state,
                fractional_scale_manager_state,
                shm_state,
                xdg_shell_state,
                xdg_decoration_state,
                xdg_dialog_state,
                xdg_activation_state,
                xdg_foreign_state,
                seat_state,
                output_manager_state,
                presentation_state,
                data_device_state,
                primary_selection_state,
                ext_data_control_state,
                wlr_data_control_state,
                xwayland_shell_state,
                xwayland_keyboard_grab_state,
                wlr_layer_shell_state,
                loop_handle: handle.clone(),
                dmabuf_state,
                dmabuf_global: None,
                drm_syncobj_state: None,
                fifo_manager_state,
                fifo_constraint_surfaces: HashSet::new(),
                commit_timing_surfaces: HashSet::new(),
                foreign_toplevel_list_state,
                image_capture_source_state,
                output_capture_source_state,
                image_copy_capture_state,
                pointer_gestures_state,
                pointer_constraints_state,
                pointer_warp_manager,
                relative_pointer_manager_state,
                single_pixel_buffer_state,
                tablet_manager_state,
                viewporter_state,
                virtual_keyboard_manager_state,
                text_input_manager_state,
                input_method_manager_state,
                keyboard_shortcuts_inhibit_state,
                idle_inhibit_manager_state,
                idle_notify_manager_state,
                session_lock_manager_state,
                ext_workspace_state,
                output_management_state,
                output_power_state,
                foreign_toplevel_management_state,
                lock_state: SessionLockState::Unlocked,
                lock_surfaces: HashMap::new(),
                idle_inhibiting_surfaces: HashSet::new(),
                render_node: None,
                seat,
                keyboard,
                pointer,
                touch,
                cursor_config,
                cursor_image_status: smithay::input::pointer::CursorImageStatus::default_named(),
                cursor_icon_override: None,
                xwm: None,
                xdisplay: None,
                next_window_id: 1,
                pending_commit_clients: Vec::new(),
                geometry_sync: HashMap::new(),
                placed_border: HashMap::new(),
                native_size_hints: HashMap::new(),
                active_resize: None,
                window_index: HashMap::new(),
                window_animations: HashMap::new(),
                layout_preview_animation: crate::animation::LayoutPreviewAnimation::default(),
                layout_preview_style: crate::types::InteractionOutlineStyle::Layout,
                layout_preview_target: None,
                foreign_toplevel_handles: HashMap::new(),
                pending_warp: None,
                cursor_position_hint: None,
                runtime: WaylandRuntimeState::default(),
                command_queue: std::cell::RefCell::new(Vec::new()),
            },
        }
    }

    pub fn init_drm_syncobj(&mut self, drm_device: smithay::backend::drm::DrmDeviceFd) {
        if smithay::wayland::drm_syncobj::supports_syncobj_eventfd(&drm_device) {
            log::info!("Explicit sync (wp_linux_drm_syncobj_v1) is supported and initialized");
            self.native.drm_syncobj_state =
                Some(smithay::wayland::drm_syncobj::DrmSyncobjState::new::<Self>(
                    &self.native.display_handle,
                    drm_device,
                ));
        } else {
            log::info!("DRM device does not support syncobj eventfd; explicit sync disabled");
        }
    }

    pub fn init_dmabuf_global(&mut self, formats: Vec<Format>, egl_display: Option<&EGLDisplay>) {
        if self.native.dmabuf_global.is_some() {
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

        self.native.render_node = render_node;

        self.native.dmabuf_global = Some(if let Some(node) = self.native.render_node {
            log::info!("dmabuf: advertising zwp_linux_dmabuf_feedback_v1 v4 on node {node:?}");
            let feedback = DmabufFeedbackBuilder::new(node.dev_id(), formats)
                .build()
                .expect("DmabufFeedbackBuilder::build");
            self.native
                .dmabuf_state
                .create_global_with_default_feedback::<Self>(&self.native.display_handle, &feedback)
        } else {
            log::info!("dmabuf: no render node available, falling back to zwp_linux_dmabuf_v1 v3");
            self.native
                .dmabuf_state
                .create_global::<Self>(&self.native.display_handle, formats)
        });
    }

    /// Transfer graphics ownership to the compositor. Frames borrow graphics
    /// alongside native scene data as disjoint fields; protocol handlers borrow
    /// the root. Rust prevents a handler from running during a frame borrow.
    #[allow(unexpected_cfgs)]
    pub(crate) fn attach_graphics(&mut self, graphics: super::graphics::Graphics) {
        #[cfg(feature = "use_system_lib")]
        let mut graphics = graphics;
        #[cfg(feature = "use_system_lib")]
        graphics.with_renderer(|renderer| {
            use smithay::backend::renderer::ImportEgl;
            match renderer.bind_wl_display(&self.native.display_handle) {
                Ok(()) => log::info!("EGL wl_drm hardware-acceleration enabled"),
                Err(err) => log::debug!(
                    "EGL wl_drm not available ({}); dmabuf v4 will be used instead",
                    err
                ),
            }
        });
        self.graphics = Some(graphics);
    }

    pub(super) fn with_renderer<T>(&mut self, f: impl FnOnce(&mut GlesRenderer) -> T) -> Option<T> {
        self.graphics
            .as_mut()
            .map(|graphics| graphics.with_renderer(f))
    }

    pub(crate) fn ctx(&mut self) -> WmCtx<'_> {
        WmCtx::Wayland(crate::contexts::WmCtxWayland {
            wayland: crate::backend::wayland::WaylandBackend::new(self),
        })
    }

    /// Run after field borrows end; requiring the complete root prevents
    /// dispatch during a model or graphics borrow. Drains in the same loop
    /// iteration, without a timer or extra WM tick. Commit handlers may
    /// register new barriers safely.
    pub(crate) fn dispatch_pending_commits(&mut self) {
        use smithay::wayland::compositor::CompositorHandler;
        while !self.native.pending_commit_clients.is_empty() {
            let mut clients = std::mem::take(&mut self.native.pending_commit_clients);
            let dh = self.native.display_handle.clone();
            for client in clients.drain(..) {
                self.client_compositor_state(&client)
                    .blocker_cleared(self, &dh);
            }
            if self.native.pending_commit_clients.is_empty() {
                self.native.pending_commit_clients = clients;
            }
        }
    }

    /// Project the shared model into the Smithay space.
    pub fn sync_space(&mut self) {
        let dead_windows: Vec<WindowId> = self
            .native
            .window_index
            .iter()
            .filter_map(|(&id, w)| if !w.alive() { Some(id) } else { None })
            .collect();

        for win in dead_windows {
            let is_overlay = self
                .native
                .find_window(win)
                .and_then(|window| window.user_data().get::<WindowIdMarker>())
                .is_some_and(|marker| marker.is_overlay);
            self.remove_window_tracking(win);
            if !is_overlay {
                self.native
                    .push_command(super::super::commands::WmCommand::UnmanageWindow(win));
            }
        }

        // Purge surfaces whose underlying resource is gone, so the HashSet
        // does not grow unbounded when clients crash or destroy surfaces
        // without sending an explicit uninhibit request.
        self.native.idle_inhibiting_surfaces.retain(|s| s.alive());

        // Only recover focus when the seat focus is actually missing or dead.
        // A plain space sync must not steal focus away from a live overlay
        // surface such as fuzzel/rofi, or from any other valid keyboard target.
        let seat_focus_needs_recovery = self
            .native
            .seat
            .get_keyboard()
            .and_then(|k| k.current_focus())
            .is_none_or(|focus| !focus.alive());
        if seat_focus_needs_recovery {
            self.restore_focus_after_overlay();
        }

        let state = &self.wm.core.state;
        let updates: Vec<(WindowId, Rect)> = self
            .native
            .space
            .elements()
            .filter_map(|window| {
                let marker = window.user_data().get::<WindowIdMarker>()?;
                let client = state.model.client(marker.id)?;
                Some((marker.id, client.geo))
            })
            .collect();
        for (window_id, geo) in updates {
            // Space sync reconciles compositor state from authoritative WM
            // geometry; use a fixed policy to avoid interactive-motion
            // heuristics changing this reconciliation path.
            self.native.set_window_target_rect(
                &self.wm.core.state,
                window_id,
                geo,
                super::window::animations::WindowMoveMode::Retarget {
                    duration: self.native.default_animation_duration(&self.wm.core.state),
                },
            );
        }
        self.native.raise_unmanaged_x11_windows();
    }

    /// Set the keyboard layout.
    pub fn set_keyboard_layout(
        &mut self,
        layout: &str,
        variant: &str,
        options: Option<&str>,
        model: Option<&str>,
    ) -> Result<(), String> {
        let config = XkbConfig {
            layout,
            variant,
            options: options.map(|s| s.to_string()),
            model: model.unwrap_or(""),
            rules: "evdev",
        };

        let keyboard = self.native.keyboard.clone();
        keyboard
            .set_xkb_config(self, config)
            .map_err(|e| format!("failed to apply Wayland keyboard layout: {e}"))
    }

    /// Flush pending data to clients.
    pub fn flush(&mut self) {
        self.native.space.refresh();
        let _ = self.native.display_handle.flush_clients();
    }

    /// Notify the idle manager of user activity.
    pub fn notify_activity(&mut self) {
        self.native
            .idle_notify_manager_state
            .notify_activity(&self.native.seat);
    }

    /// Switch to a Linux virtual terminal (TTY) if running on a DRM session.
    pub fn switch_vt(&mut self, vt: i32) -> bool {
        if let Some(session) = self.native.runtime.session.as_mut() {
            log::info!("Switching to VT {vt}");
            if let Err(err) = smithay::backend::session::Session::change_vt(session, vt) {
                log::error!("Failed to switch to VT {vt}: {err}");
                false
            } else {
                true
            }
        } else {
            false
        }
    }
}

impl WaylandNativeState {
    /// Barrier signaling may happen during a frame or shared WM work. Smithay's
    /// blocker_cleared synchronously invokes CompositorHandler::commit, which
    /// can consult the model (native menu placement) or graphics. Keep that
    /// callback outside all WM/graphics borrows instead of relying on each
    /// caller to know which protocol handlers are reentrant.
    pub(crate) fn defer_commit_client(
        &mut self,
        client: smithay::reexports::wayland_server::Client,
    ) {
        if !self
            .pending_commit_clients
            .iter()
            .any(|pending| pending.id() == client.id())
        {
            self.pending_commit_clients.push(client);
        }
    }
    /// Returns `true` if the session is currently locked.
    pub fn is_locked(&self) -> bool {
        matches!(self.lock_state, SessionLockState::Locked(_))
    }
}

impl WaylandNativeState {
    pub fn push_command(&self, command: super::super::commands::WmCommand) {
        self.command_queue.borrow_mut().push(command);
    }
}

#[cfg(test)]
mod native_menu_tests {
    use std::time::Instant;

    use crate::systray::status_notifier::NativeMenuRequest;
    use crate::types::Point;

    #[test]
    fn pending_native_menu_identity_is_available_until_dismissal() {
        let (_event_loop, mut state) = crate::test_support::new_compositor();
        *state.native.runtime.pending_systray_menu.lock().unwrap() = Some(NativeMenuRequest {
            created: Instant::now(),
            anchor: Point::new(10, 20),
            service: "org.example.Tray".into(),
            path: "/org/example/Tray".into(),
            owner_pid: Some(42),
        });

        assert!(state.native_systray_menu_matches("org.example.Tray", "/org/example/Tray"));
        assert!(!state.native_systray_menu_matches("org.example.Other", "/org/example/Tray"));
        assert!(state.dismiss_native_systray_menu());
        assert!(!state.native_systray_menu_matches("org.example.Tray", "/org/example/Tray"));
    }
}
