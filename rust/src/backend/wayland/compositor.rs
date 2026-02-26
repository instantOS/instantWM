//! Smithay compositor state and protocol handler implementations.
//!
//! This is the heart of the Wayland backend.  `WaylandState` owns all
//! Smithay protocol state objects and implements every handler trait that
//! Smithay requires.
//!
//! # How to use this module
//!
//! ```ignore
//! use crate::backend::wayland::compositor::WaylandState;
//!
//! let event_loop = calloop::EventLoop::try_new().unwrap();
//! let state = WaylandState::new(&event_loop.handle());
//! // insert sources, run loop…
//! ```
//!
//! # Smithay patterns used here
//!
//! Each Wayland protocol global follows a three-step pattern:
//!
//! 1. **State struct** — stored as a field on `WaylandState`.
//! 2. **Handler trait** — implemented on `WaylandState`.
//! 3. **delegate macro** — generates `wayland_server::Dispatch` impls.
//!
//! The `delegate_*!` macros MUST be called at the module level (not inside
//! an `impl` block).  They wire Smithay's internal message routing to the
//! handler trait implementation.

use std::sync::Arc;

use smithay::{
    delegate_compositor, delegate_output, delegate_seat, delegate_shm, delegate_xdg_shell,
    delegate_xwayland_shell,
    desktop::{PopupManager, Space, Window},
    input::{
        keyboard::{KeyState, KeysymHandle, ModifiersState, XkbConfig},
        Seat, SeatHandler, SeatState,
    },
    output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{generic::Generic, Interest, LoopHandle, Mode, PostAction},
        wayland_server::{
            backend::ClientData,
            protocol::{wl_seat, wl_surface::WlSurface},
            Client, Display, DisplayHandle,
        },
    },
    utils::{IsAlive, Scale, Serial, Transform, SERIAL_COUNTER},
    wayland::{
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        output::OutputManagerState,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
        shm::{ShmHandler, ShmState},
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::X11Wm,
};

// ---------------------------------------------------------------------------
// Focus target types
// ---------------------------------------------------------------------------

/// What can receive keyboard focus in the compositor.
///
/// `Window` already wraps both Wayland `ToplevelSurface` and XWayland
/// `X11Surface`, so a single variant suffices for most cases.  Layer
/// surfaces and popups will be added as features are implemented.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyboardFocusTarget {
    Window(Window),
}

/// What can receive pointer focus.
///
/// An explicit `WlSurface` variant is needed because pointer events
/// target individual surfaces (e.g. subsurfaces within a window).
#[derive(Debug, Clone, PartialEq)]
pub enum PointerFocusTarget {
    WlSurface(WlSurface),
}

// -- IsAlive implementations (required by KeyboardTarget / PointerTarget) --

impl IsAlive for KeyboardFocusTarget {
    fn alive(&self) -> bool {
        match self {
            KeyboardFocusTarget::Window(w) => w.alive(),
        }
    }
}

impl IsAlive for PointerFocusTarget {
    fn alive(&self) -> bool {
        match self {
            PointerFocusTarget::WlSurface(s) => s.alive(),
        }
    }
}

// -- KeyboardTarget implementation --

impl smithay::input::keyboard::KeyboardTarget<WaylandState> for KeyboardFocusTarget {
    fn enter(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        match self {
            KeyboardFocusTarget::Window(w) => {
                if let Some(surface) = w.wl_surface() {
                    smithay::input::keyboard::KeyboardTarget::enter(
                        &surface, seat, data, keys, serial,
                    );
                }
            }
        }
    }

    fn leave(&self, seat: &Seat<WaylandState>, data: &mut WaylandState, serial: Serial) {
        match self {
            KeyboardFocusTarget::Window(w) => {
                if let Some(surface) = w.wl_surface() {
                    smithay::input::keyboard::KeyboardTarget::leave(&surface, seat, data, serial);
                }
            }
        }
    }

    fn key(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: u32,
    ) {
        match self {
            KeyboardFocusTarget::Window(w) => {
                if let Some(surface) = w.wl_surface() {
                    smithay::input::keyboard::KeyboardTarget::key(
                        &surface, seat, data, key, state, serial, time,
                    );
                }
            }
        }
    }

    fn modifiers(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        modifiers: ModifiersState,
        serial: Serial,
    ) {
        match self {
            KeyboardFocusTarget::Window(w) => {
                if let Some(surface) = w.wl_surface() {
                    smithay::input::keyboard::KeyboardTarget::modifiers(
                        &surface, seat, data, modifiers, serial,
                    );
                }
            }
        }
    }
}

// -- PointerTarget implementation --

impl smithay::input::pointer::PointerTarget<WaylandState> for PointerFocusTarget {
    fn enter(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::MotionEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::enter(s, seat, data, event);
            }
        }
    }

    fn motion(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::MotionEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::motion(s, seat, data, event);
            }
        }
    }

    fn relative_motion(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::RelativeMotionEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::relative_motion(s, seat, data, event);
            }
        }
    }

    fn button(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::ButtonEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::button(s, seat, data, event);
            }
        }
    }

    fn axis(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        frame: smithay::input::pointer::AxisFrame,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::axis(s, seat, data, frame);
            }
        }
    }

    fn frame(&self, seat: &Seat<WaylandState>, data: &mut WaylandState) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::frame(s, seat, data);
            }
        }
    }

    fn gesture_swipe_begin(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GestureSwipeBeginEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::gesture_swipe_begin(s, seat, data, event);
            }
        }
    }

    fn gesture_swipe_update(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GestureSwipeUpdateEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::gesture_swipe_update(s, seat, data, event);
            }
        }
    }

    fn gesture_swipe_end(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GestureSwipeEndEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::gesture_swipe_end(s, seat, data, event);
            }
        }
    }

    fn gesture_pinch_begin(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GesturePinchBeginEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::gesture_pinch_begin(s, seat, data, event);
            }
        }
    }

    fn gesture_pinch_update(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GesturePinchUpdateEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::gesture_pinch_update(s, seat, data, event);
            }
        }
    }

    fn gesture_pinch_end(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GesturePinchEndEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::gesture_pinch_end(s, seat, data, event);
            }
        }
    }

    fn gesture_hold_begin(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GestureHoldBeginEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::gesture_hold_begin(s, seat, data, event);
            }
        }
    }

    fn gesture_hold_end(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GestureHoldEndEvent,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::gesture_hold_end(s, seat, data, event);
            }
        }
    }

    fn leave(&self, seat: &Seat<WaylandState>, data: &mut WaylandState, serial: Serial, time: u32) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::leave(s, seat, data, serial, time);
            }
        }
    }
}

// -- TouchTarget implementation --

impl smithay::input::touch::TouchTarget<WaylandState> for PointerFocusTarget {
    fn down(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::touch::DownEvent,
        seq: Serial,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::touch::TouchTarget::down(s, seat, data, event, seq);
            }
        }
    }

    fn up(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::touch::UpEvent,
        seq: Serial,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::touch::TouchTarget::up(s, seat, data, event, seq);
            }
        }
    }

    fn motion(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::touch::MotionEvent,
        seq: Serial,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::touch::TouchTarget::motion(s, seat, data, event, seq);
            }
        }
    }

    fn frame(&self, seat: &Seat<WaylandState>, data: &mut WaylandState, seq: Serial) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::touch::TouchTarget::frame(s, seat, data, seq);
            }
        }
    }

    fn cancel(&self, seat: &Seat<WaylandState>, data: &mut WaylandState, seq: Serial) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::touch::TouchTarget::cancel(s, seat, data, seq);
            }
        }
    }

    fn shape(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::touch::ShapeEvent,
        seq: Serial,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::touch::TouchTarget::shape(s, seat, data, event, seq);
            }
        }
    }

    fn orientation(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::touch::OrientationEvent,
        seq: Serial,
    ) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::touch::TouchTarget::orientation(s, seat, data, event, seq);
            }
        }
    }
}

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
    pub compositor_state: CompositorClientState,
}

impl ClientData for WaylandClientState {
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
///
/// # Fields by category
///
/// ## Wayland infrastructure
/// - `display_handle` — cheap clone of the `Display` handle for registering
///   globals and inserting clients.
///
/// ## Desktop abstractions
/// - `space` — Smithay's 2D workspace plane; maps windows and outputs at
///   logical coordinates and handles hit-testing.
/// - `popups` — popup manager for xdg-popup tracking.
///
/// ## Protocol states (one per Wayland global)
/// - `compositor_state`, `shm_state`, `xdg_shell_state`, `seat_state`,
///   `output_manager_state`.  More will be added as protocols are needed.
///
/// ## Input
/// - `seat` — the compositor's input seat (keyboard + pointer + touch).
///
/// ## XWayland
/// - `xwayland_shell_state`, `xwm`, `xdisplay` — XWayland integration.
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
    pub seat_state: SeatState<WaylandState>,
    pub output_manager_state: OutputManagerState,
    pub xwayland_shell_state: XWaylandShellState,

    // -- Input --
    pub seat: Seat<WaylandState>,

    // -- XWayland --
    pub xwm: Option<X11Wm>,
    pub xdisplay: Option<u32>,
}

impl WaylandState {
    /// Create a new `WaylandState` and register all Wayland globals.
    ///
    /// This follows Smithay's Anvil pattern:
    /// 1. Create a `Display` and extract its `DisplayHandle`.
    /// 2. Insert the display as a calloop source for dispatching.
    /// 3. Create each protocol state with `FooState::new::<WaylandState>(&dh)`.
    /// 4. Create a seat and add input devices.
    /// 5. Create at least one output.
    ///
    /// The caller is responsible for creating the `EventLoop` and running it.
    pub fn new(display: Display<WaylandState>, handle: &LoopHandle<'static, WaylandState>) -> Self {
        let dh = display.handle();

        // Insert the Wayland display as a calloop source so that protocol
        // messages from connected clients are dispatched on each loop tick.
        handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, data| {
                    unsafe {
                        display.get_mut().dispatch_clients(data).unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .expect("Failed to insert Wayland display source");

        // -- Protocol globals --
        let compositor_state = CompositorState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let xwayland_shell_state = XWaylandShellState::new::<Self>(&dh);

        // -- Seat (input devices) --
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "seat-0");
        seat.add_keyboard(XkbConfig::default(), 200, 25)
            .expect("Failed to add keyboard to seat");
        let _pointer = seat.add_pointer();

        WaylandState {
            display_handle: dh,
            space: Space::default(),
            popups: PopupManager::default(),
            compositor_state,
            shm_state,
            xdg_shell_state,
            seat_state,
            output_manager_state,
            xwayland_shell_state,
            seat,
            xwm: None,
            xdisplay: None,
        }
    }

    /// Create and register a default output.
    ///
    /// Call this after construction to set up an initial output that
    /// matches the physical display (or a default for testing).
    pub fn create_output(&mut self, name: &str, width: i32, height: i32) -> Output {
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
            size: (width, height).into(),
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
}

// ---------------------------------------------------------------------------
// Protocol handler implementations
// ---------------------------------------------------------------------------

impl CompositorHandler for WaylandState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client
            .get_data::<WaylandClientState>()
            .expect("client missing WaylandClientState")
            .compositor_state
    }

    fn commit(&mut self, _surface: &WlSurface) {
        // TODO: walk surface tree, send initial configure for new toplevels,
        // refresh Space, schedule repaint.
    }
}

impl ShmHandler for WaylandState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl SeatHandler for WaylandState {
    type KeyboardFocus = KeyboardFocusTarget;
    type PointerFocus = PointerFocusTarget;
    type TouchFocus = PointerFocusTarget;

    fn seat_state(&mut self) -> &mut SeatState<WaylandState> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _target: Option<&KeyboardFocusTarget>) {
        // TODO: update data device focus for clipboard bridging.
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
        // TODO: store cursor image for rendering.
    }
}

impl XdgShellHandler for WaylandState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Wrap the toplevel in a Smithay Window and map it into the space.
        // Smithay will send the initial configure on first commit.
        let window = Window::new_wayland_window(surface);
        self.space.map_element(window, (0, 0), true);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        let _ = self
            .popups
            .track_popup(smithay::desktop::PopupKind::Xdg(surface));
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface();
        if let Some(window) = self
            .space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(wl_surface))
            .cloned()
        {
            self.space.unmap_elem(&window);
        }
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        // TODO: implement popup grab.
    }

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
        // TODO: reposition popup.
    }

    fn move_request(&mut self, _surface: ToplevelSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        // TODO: initiate interactive move (pointer grab).
    }

    fn resize_request(
        &mut self,
        _surface: ToplevelSurface,
        _seat: wl_seat::WlSeat,
        _serial: Serial,
        _edges: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    ) {
        // TODO: initiate interactive resize (pointer grab).
    }
}

impl XWaylandShellHandler for WaylandState {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl smithay::wayland::output::OutputHandler for WaylandState {}

// ---------------------------------------------------------------------------
// Delegate macros — these MUST be at module level
// ---------------------------------------------------------------------------

delegate_compositor!(WaylandState);
delegate_shm!(WaylandState);
delegate_seat!(WaylandState);
delegate_xdg_shell!(WaylandState);
delegate_output!(WaylandState);
delegate_xwayland_shell!(WaylandState);
