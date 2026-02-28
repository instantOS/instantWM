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

use std::borrow::Cow;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};

use smithay::{
    backend::{input::KeyState, renderer::utils::on_commit_buffer_handler},
    delegate_compositor, delegate_data_device, delegate_output, delegate_seat, delegate_shm,
    delegate_xdg_shell, delegate_xwayland_shell,
    desktop::{PopupKind, PopupManager, Space, Window},
    input::{
        keyboard::{KeyboardHandle, KeysymHandle, ModifiersState, XkbConfig},
        pointer::PointerHandle,
        Seat, SeatHandler, SeatState,
    },
    output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        calloop::{generic::Generic, Interest, LoopHandle, Mode, PostAction},
        wayland_server::{
            backend::ClientData,
            protocol::{wl_seat, wl_surface::WlSurface},
            Client, Display, DisplayHandle,
        },
    },
    utils::{IsAlive, Serial, Transform, SERIAL_COUNTER},
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        output::OutputManagerState,
        seat::WaylandFocus,
        selection::{
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
            },
            SelectionHandler,
        },
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
        shm::{ShmHandler, ShmState},
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::X11Wm,
};

use crate::globals::Globals;
use crate::types::{Client as WmClient, Rect, WindowId};
use std::ptr::NonNull;

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
    WlSurface(WlSurface),
    Popup(PopupKind),
}

impl From<PopupKind> for KeyboardFocusTarget {
    fn from(kind: PopupKind) -> Self {
        KeyboardFocusTarget::Popup(kind)
    }
}

/// What can receive pointer focus.
///
/// An explicit `WlSurface` variant is needed because pointer events
/// target individual surfaces (e.g. subsurfaces within a window).
#[derive(Debug, Clone, PartialEq)]
pub enum PointerFocusTarget {
    WlSurface(WlSurface),
    Popup(PopupKind),
}

impl From<PopupKind> for PointerFocusTarget {
    fn from(kind: PopupKind) -> Self {
        PointerFocusTarget::Popup(kind)
    }
}

impl From<KeyboardFocusTarget> for PointerFocusTarget {
    fn from(target: KeyboardFocusTarget) -> Self {
        match target {
            KeyboardFocusTarget::Window(w) => {
                PointerFocusTarget::WlSurface(w.wl_surface().unwrap().into_owned())
            }
            KeyboardFocusTarget::WlSurface(s) => PointerFocusTarget::WlSurface(s),
            KeyboardFocusTarget::Popup(p) => PointerFocusTarget::Popup(p),
        }
    }
}

// -- IsAlive implementations (required by KeyboardTarget / PointerTarget) --

impl IsAlive for KeyboardFocusTarget {
    fn alive(&self) -> bool {
        match self {
            KeyboardFocusTarget::Window(w) => w.alive(),
            KeyboardFocusTarget::WlSurface(s) => s.alive(),
            KeyboardFocusTarget::Popup(p) => p.alive(),
        }
    }
}

impl IsAlive for PointerFocusTarget {
    fn alive(&self) -> bool {
        match self {
            PointerFocusTarget::WlSurface(s) => s.alive(),
            PointerFocusTarget::Popup(p) => p.alive(),
        }
    }
}

// -- WaylandFocus implementations --

impl WaylandFocus for KeyboardFocusTarget {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            KeyboardFocusTarget::Window(w) => w.wl_surface(),
            KeyboardFocusTarget::WlSurface(s) => Some(Cow::Borrowed(s)),
            KeyboardFocusTarget::Popup(p) => Some(Cow::Borrowed(p.wl_surface())),
        }
    }
}

impl WaylandFocus for PointerFocusTarget {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            PointerFocusTarget::WlSurface(s) => Some(Cow::Borrowed(s)),
            PointerFocusTarget::Popup(p) => Some(Cow::Borrowed(p.wl_surface())),
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
                        &surface.into_owned(),
                        seat,
                        data,
                        keys,
                        serial,
                    );
                }
            }
            KeyboardFocusTarget::WlSurface(surface) => {
                smithay::input::keyboard::KeyboardTarget::enter(surface, seat, data, keys, serial);
            }
            KeyboardFocusTarget::Popup(p) => {
                smithay::input::keyboard::KeyboardTarget::enter(
                    p.wl_surface(),
                    seat,
                    data,
                    keys,
                    serial,
                );
            }
        }
    }

    fn leave(&self, seat: &Seat<WaylandState>, data: &mut WaylandState, serial: Serial) {
        match self {
            KeyboardFocusTarget::Window(w) => {
                if let Some(surface) = w.wl_surface() {
                    smithay::input::keyboard::KeyboardTarget::leave(
                        &surface.into_owned(),
                        seat,
                        data,
                        serial,
                    );
                }
            }
            KeyboardFocusTarget::WlSurface(surface) => {
                smithay::input::keyboard::KeyboardTarget::leave(surface, seat, data, serial);
            }
            KeyboardFocusTarget::Popup(p) => {
                smithay::input::keyboard::KeyboardTarget::leave(p.wl_surface(), seat, data, serial);
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
                        &surface.into_owned(),
                        seat,
                        data,
                        key,
                        state,
                        serial,
                        time,
                    );
                }
            }
            KeyboardFocusTarget::WlSurface(surface) => {
                smithay::input::keyboard::KeyboardTarget::key(
                    surface, seat, data, key, state, serial, time,
                );
            }
            KeyboardFocusTarget::Popup(p) => {
                smithay::input::keyboard::KeyboardTarget::key(
                    p.wl_surface(),
                    seat,
                    data,
                    key,
                    state,
                    serial,
                    time,
                );
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
                        &surface.into_owned(),
                        seat,
                        data,
                        modifiers,
                        serial,
                    );
                }
            }
            KeyboardFocusTarget::WlSurface(surface) => {
                smithay::input::keyboard::KeyboardTarget::modifiers(
                    surface, seat, data, modifiers, serial,
                );
            }
            KeyboardFocusTarget::Popup(p) => {
                smithay::input::keyboard::KeyboardTarget::modifiers(
                    p.wl_surface(),
                    seat,
                    data,
                    modifiers,
                    serial,
                );
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::pointer::PointerTarget::enter(p.wl_surface(), seat, data, event);
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::pointer::PointerTarget::motion(p.wl_surface(), seat, data, event);
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::pointer::PointerTarget::relative_motion(
                    p.wl_surface(),
                    seat,
                    data,
                    event,
                );
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::pointer::PointerTarget::button(p.wl_surface(), seat, data, event);
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::pointer::PointerTarget::axis(p.wl_surface(), seat, data, frame);
            }
        }
    }

    fn frame(&self, seat: &Seat<WaylandState>, data: &mut WaylandState) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::frame(s, seat, data);
            }
            PointerFocusTarget::Popup(p) => {
                smithay::input::pointer::PointerTarget::frame(p.wl_surface(), seat, data);
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::pointer::PointerTarget::gesture_swipe_begin(
                    p.wl_surface(),
                    seat,
                    data,
                    event,
                );
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::pointer::PointerTarget::gesture_swipe_update(
                    p.wl_surface(),
                    seat,
                    data,
                    event,
                );
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::pointer::PointerTarget::gesture_swipe_end(
                    p.wl_surface(),
                    seat,
                    data,
                    event,
                );
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::pointer::PointerTarget::gesture_pinch_begin(
                    p.wl_surface(),
                    seat,
                    data,
                    event,
                );
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::pointer::PointerTarget::gesture_pinch_update(
                    p.wl_surface(),
                    seat,
                    data,
                    event,
                );
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::pointer::PointerTarget::gesture_pinch_end(
                    p.wl_surface(),
                    seat,
                    data,
                    event,
                );
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::pointer::PointerTarget::gesture_hold_begin(
                    p.wl_surface(),
                    seat,
                    data,
                    event,
                );
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::pointer::PointerTarget::gesture_hold_end(
                    p.wl_surface(),
                    seat,
                    data,
                    event,
                );
            }
        }
    }

    fn leave(&self, seat: &Seat<WaylandState>, data: &mut WaylandState, serial: Serial, time: u32) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::pointer::PointerTarget::leave(s, seat, data, serial, time);
            }
            PointerFocusTarget::Popup(p) => {
                smithay::input::pointer::PointerTarget::leave(
                    p.wl_surface(),
                    seat,
                    data,
                    serial,
                    time,
                );
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::touch::TouchTarget::down(p.wl_surface(), seat, data, event, seq);
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::touch::TouchTarget::up(p.wl_surface(), seat, data, event, seq);
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::touch::TouchTarget::motion(p.wl_surface(), seat, data, event, seq);
            }
        }
    }

    fn frame(&self, seat: &Seat<WaylandState>, data: &mut WaylandState, seq: Serial) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::touch::TouchTarget::frame(s, seat, data, seq);
            }
            PointerFocusTarget::Popup(p) => {
                smithay::input::touch::TouchTarget::frame(p.wl_surface(), seat, data, seq);
            }
        }
    }

    fn cancel(&self, seat: &Seat<WaylandState>, data: &mut WaylandState, seq: Serial) {
        match self {
            PointerFocusTarget::WlSurface(s) => {
                smithay::input::touch::TouchTarget::cancel(s, seat, data, seq);
            }
            PointerFocusTarget::Popup(p) => {
                smithay::input::touch::TouchTarget::cancel(p.wl_surface(), seat, data, seq);
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::touch::TouchTarget::shape(p.wl_surface(), seat, data, event, seq);
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
            PointerFocusTarget::Popup(p) => {
                smithay::input::touch::TouchTarget::orientation(
                    p.wl_surface(),
                    seat,
                    data,
                    event,
                    seq,
                );
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
    pub data_device_state: DataDeviceState,
    pub xwayland_shell_state: XWaylandShellState,

    // -- Input --
    pub seat: Seat<WaylandState>,
    pub keyboard: KeyboardHandle<WaylandState>,
    pub pointer: PointerHandle<WaylandState>,

    // -- XWayland --
    pub xwm: Option<X11Wm>,
    pub xdisplay: Option<u32>,

    next_window_id: u32,
    globals: Option<NonNull<Globals>>,
    last_configured_size: HashMap<WindowId, (i32, i32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowIdMarker(pub WindowId);

impl WaylandState {
    const MIN_WL_DIM: i32 = 64;
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
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let xwayland_shell_state = XWaylandShellState::new::<Self>(&dh);

        // -- Seat (input devices) --
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "seat-0");
        let keyboard = seat
            .add_keyboard(XkbConfig::default(), 200, 25)
            .expect("Failed to add keyboard to seat");
        let pointer = seat.add_pointer();

        WaylandState {
            display_handle: dh,
            space: Space::default(),
            popups: PopupManager::default(),
            compositor_state,
            shm_state,
            xdg_shell_state,
            seat_state,
            output_manager_state,
            data_device_state,
            xwayland_shell_state,
            seat,
            keyboard,
            pointer,
            xwm: None,
            xdisplay: None,
            next_window_id: 1,
            globals: None,
            last_configured_size: HashMap::new(),
        }
    }

    pub fn attach_globals(&mut self, globals: &mut Globals) {
        self.globals = Some(NonNull::from(globals));
    }

    #[inline]
    fn globals(&self) -> Option<&Globals> {
        self.globals.map(|p| unsafe { p.as_ref() })
    }

    #[inline]
    fn globals_mut(&mut self) -> Option<&mut Globals> {
        self.globals.map(|mut p| unsafe { p.as_mut() })
    }

    /// Create and register a default output.
    ///
    /// Call this after construction to set up an initial output that
    /// matches the physical display (or a default for testing).
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
            // Keep Flipped180: required for this backend's output orientation,
            // consistent with the official Smithay demo compositor setup.
            Some(Transform::Flipped180),
            Some(Scale::Integer(1)),
            Some((0, 0).into()),
        );
        output.set_preferred(mode);

        let _global = output.create_global::<WaylandState>(&self.display_handle);
        self.space.map_output(&output, (0, 0));

        output
    }

    pub fn sync_space_from_globals(&mut self) {
        let Some(g) = self.globals() else {
            return;
        };
        let updates: Vec<(Window, Rect, i32)> = self
            .space
            .elements()
            .filter_map(|window| {
                let marker = window.user_data().get::<WindowIdMarker>()?;
                let client = g.clients.get(&marker.0)?;
                Some((window.clone(), client.geo, client.border_width))
            })
            .collect();
        for (window, geo, bw) in updates {
            // Offset by border_width: content sits inside the drawn border.
            self.space
                .map_element(window.clone(), (geo.x + bw, geo.y + bw), false);
            if let Some(toplevel) = window.toplevel() {
                let key = window
                    .user_data()
                    .get::<WindowIdMarker>()
                    .map(|m| m.0)
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
    }

    pub fn map_new_toplevel(&mut self, surface: ToplevelSurface) -> WindowId {
        let window = Window::new_wayland_window(surface);
        let window_id = self.alloc_window_id();
        let _ = window
            .user_data()
            .get_or_insert_threadsafe(|| WindowIdMarker(window_id));

        self.space.map_element(window.clone(), (0, 0), true);
        self.ensure_client_for_window(window_id);
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
        self.set_focus(window_id);
        window_id
    }

    pub fn resize_window(&mut self, window: WindowId, rect: Rect) {
        if let Some(element) = self.find_window(window).cloned() {
            // In X11, the server draws borders outside the client area.
            // In Wayland we draw borders ourselves, so we offset the surface
            // position by border_width so the content sits inside the border.
            let bw = self
                .globals()
                .and_then(|g| g.clients.get(&window).map(|c| c.border_width))
                .unwrap_or(0);
            self.space
                .map_element(element.clone(), (rect.x + bw, rect.y + bw), false);
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
    }

    pub fn restack(&mut self, windows: &[WindowId]) {
        for window in windows {
            if let Some(element) = self.find_window(*window).cloned() {
                self.space.raise_element(&element, false);
            }
        }
    }

    pub fn set_focus(&mut self, window: WindowId) {
        let serial = SERIAL_COUNTER.next_serial();
        let focus = self
            .find_window(window)
            .cloned()
            .map(KeyboardFocusTarget::Window);
        let windows = self.space.elements().cloned().collect::<Vec<_>>();
        for w in windows {
            let is_target = w.user_data().get::<WindowIdMarker>().map(|m| m.0) == Some(window);
            if w.set_activated(is_target) {
                if let Some(toplevel) = w.toplevel() {
                    toplevel.send_pending_configure();
                }
            }
        }
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, focus, serial);
        }
    }

    pub fn close_window(&mut self, window: WindowId) -> bool {
        let Some(element) = self.find_window(window).cloned() else {
            return false;
        };
        if let Some(toplevel) = element.toplevel() {
            toplevel.send_close();
            return true;
        }
        false
    }

    pub fn map_window(&mut self, window: WindowId) {
        if let Some(element) = self.find_window(window).cloned() {
            let loc = self
                .space
                .element_location(&element)
                .unwrap_or((0, 0).into());
            self.space.map_element(element, loc, false);
        }
    }

    pub fn unmap_window(&mut self, window: WindowId) {
        if let Some(element) = self.find_window(window).cloned() {
            self.space.unmap_elem(&element);
        }
        self.last_configured_size.remove(&window);
    }

    pub fn flush(&mut self) {
        self.space.refresh();
        let _ = self.display_handle.flush_clients();
    }

    pub fn window_exists(&self, window: WindowId) -> bool {
        self.find_window(window).is_some()
    }

    fn alloc_window_id(&mut self) -> WindowId {
        loop {
            let id = self.next_window_id;
            self.next_window_id = self.next_window_id.wrapping_add(1).max(1);
            let window_id = WindowId::from(id);
            if !self
                .space
                .elements()
                .any(|w| w.user_data().get::<WindowIdMarker>().map(|m| m.0) == Some(window_id))
            {
                return window_id;
            }
        }
    }

    fn find_window(&self, window: WindowId) -> Option<&Window> {
        self.space
            .elements()
            .find(|w| w.user_data().get::<WindowIdMarker>().map(|m| m.0) == Some(window))
    }

    fn ensure_client_for_window(&mut self, window: WindowId) {
        let Some(g) = self.globals_mut() else {
            return;
        };
        if g.clients.contains_key(&window) {
            return;
        }

        let mon_id = g.selmon_id();
        let (base_w, base_h) = g
            .monitor(mon_id)
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
        c.border_width = g.cfg.borderpx;
        c.old_border_width = g.cfg.borderpx;
        c.tags = 1;
        c.mon_id = Some(mon_id);
        g.clients.insert(window, c);
        g.client_list.push(window.0 as usize);
        attach_client_to_monitor(g, window);
    }

    fn window_id_for_toplevel(&self, surface: &ToplevelSurface) -> Option<WindowId> {
        let wl_surface = surface.wl_surface();
        self.space.elements().find_map(|w| {
            if w.wl_surface().as_deref() == Some(wl_surface) {
                w.user_data().get::<WindowIdMarker>().map(|m| m.0)
            } else {
                None
            }
        })
    }
}

fn attach_client_to_monitor(g: &mut Globals, win: WindowId) {
    let mon_id = match g.clients.get(&win).and_then(|c| c.mon_id) {
        Some(mid) => mid,
        None => return,
    };
    let old_clients = g.monitor(mon_id).and_then(|m| m.clients);
    let old_stack = g.monitor(mon_id).and_then(|m| m.stack);
    if let Some(c) = g.clients.get_mut(&win) {
        c.next = old_clients;
        c.snext = old_stack;
    }
    if let Some(mon) = g.monitor_mut(mon_id) {
        mon.clients = Some(win);
        mon.stack = Some(win);
        if mon.sel.is_none() {
            mon.sel = Some(win);
        }
    }
}

fn detach_client_from_monitor(g: &mut Globals, win: WindowId) {
    let mon_id = match g.clients.get(&win).and_then(|c| c.mon_id) {
        Some(mid) => mid,
        None => return,
    };
    let client_next = g.clients.get(&win).and_then(|c| c.next);
    let client_snext = g.clients.get(&win).and_then(|c| c.snext);

    let mut cur = g.monitor(mon_id).and_then(|m| m.clients);
    let mut prev: Option<WindowId> = None;
    while let Some(w) = cur {
        let next = g.clients.get(&w).and_then(|c| c.next);
        if w == win {
            if let Some(p) = prev {
                if let Some(pc) = g.clients.get_mut(&p) {
                    pc.next = client_next;
                }
            } else if let Some(mon) = g.monitor_mut(mon_id) {
                mon.clients = client_next;
            }
            break;
        }
        prev = Some(w);
        cur = next;
    }

    let mut cur = g.monitor(mon_id).and_then(|m| m.stack);
    let mut prev: Option<WindowId> = None;
    while let Some(w) = cur {
        let next = g.clients.get(&w).and_then(|c| c.snext);
        if w == win {
            if let Some(p) = prev {
                if let Some(pc) = g.clients.get_mut(&p) {
                    pc.snext = client_snext;
                }
            } else if let Some(mon) = g.monitor_mut(mon_id) {
                mon.stack = client_snext;
            }
            break;
        }
        prev = Some(w);
        cur = next;
    }

    if let Some(mon) = g.monitor_mut(mon_id) {
        if mon.sel == Some(win) {
            mon.sel = mon.clients;
        }
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

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        let _ = self.popups.commit(surface);
        if let Some(window) = self
            .space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(surface))
            .cloned()
        {
            window.on_commit();
        }
    }
}

impl SelectionHandler for WaylandState {
    type SelectionUserData = ();
}

impl DataDeviceHandler for WaylandState {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for WaylandState {}
impl ServerDndGrabHandler for WaylandState {
    fn send(&mut self, _mime_type: String, _fd: std::os::unix::io::OwnedFd, _seat: Seat<Self>) {}
}

impl ShmHandler for WaylandState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl BufferHandler for WaylandState {
    fn buffer_destroyed(
        &mut self,
        _buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    ) {
    }
}

impl smithay::xwayland::XwmHandler for WaylandState {
    fn xwm_state(&mut self, _xwm: smithay::xwayland::xwm::XwmId) -> &mut X11Wm {
        self.xwm.as_mut().expect("XWayland is not initialized")
    }

    fn new_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
    ) {
    }

    fn new_override_redirect_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
    ) {
    }

    fn map_window_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
    ) {
    }

    fn mapped_override_redirect_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
    ) {
    }

    fn unmapped_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
    ) {
    }

    fn destroyed_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
    ) {
    }

    fn configure_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
        _x: Option<i32>,
        _y: Option<i32>,
        _w: Option<u32>,
        _h: Option<u32>,
        _reorder: Option<smithay::xwayland::xwm::Reorder>,
    ) {
    }

    fn configure_notify(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
        _geometry: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
        _above: Option<smithay::xwayland::xwm::X11Window>,
    ) {
    }

    fn resize_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
        _button: u32,
        _resize_edge: smithay::xwayland::xwm::ResizeEdge,
    ) {
    }

    fn move_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        _window: smithay::xwayland::X11Surface,
        _button: u32,
    ) {
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
        let _ = self.map_new_toplevel(surface);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        let kind = smithay::desktop::PopupKind::Xdg(surface);
        let _ = self
            .popups
            .track_popup(kind.clone());
        let _ = self.popups.commit(kind.wl_surface());
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface();
        let windows = self.space.elements().cloned().collect::<Vec<_>>();
        let mut destroyed_win: Option<WindowId> = None;
        if let Some(window) = windows
            .into_iter()
            .find(|w| w.wl_surface().as_deref() == Some(wl_surface))
        {
            self.space.unmap_elem(&window);
            destroyed_win = window
                .user_data()
                .get::<WindowIdMarker>()
                .map(|m| m.0);
        }
        let Some(win) = destroyed_win else { return };
        self.last_configured_size.remove(&win);
        let new_sel = {
            let Some(g) = self.globals_mut() else {
                return;
            };
            if g.clients.contains_key(&win) {
                detach_client_from_monitor(g, win);
                g.clients.remove(&win);
                g.client_list.retain(|id| *id != win.0 as usize);
            }
            g.selected_win()
        };
        // Update Smithay keyboard focus to match mon.sel.
        if let Some(new_win) = new_sel {
            self.set_focus(new_win);
        } else {
            let serial = SERIAL_COUNTER.next_serial();
            if let Some(keyboard) = self.seat.get_keyboard() {
                keyboard.set_focus(self, None::<KeyboardFocusTarget>, serial);
            }
        }
    }

    fn grab(&mut self, surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        let kind = smithay::desktop::PopupKind::Xdg(surface.clone());
        if let Some(parent) = surface.parent_surface() {
            let ret = self.popups.grab_popup(kind, parent, &self.seat, _serial);
            if let Err(err) = ret {
                log::warn!("Failed to grab popup: {:?}", err);
            }
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        _positioner: PositionerState,
        token: u32,
    ) {
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        if let Some(win) = self.window_id_for_toplevel(&surface) {
            self.set_focus(win);
            self.raise_window(win);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        _seat: wl_seat::WlSeat,
        _serial: Serial,
        _edges: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    ) {
        if let Some(win) = self.window_id_for_toplevel(&surface) {
            self.set_focus(win);
            self.raise_window(win);
        }
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
delegate_data_device!(WaylandState);
delegate_shm!(WaylandState);
delegate_seat!(WaylandState);
delegate_xdg_shell!(WaylandState);
delegate_output!(WaylandState);
delegate_xwayland_shell!(WaylandState);
