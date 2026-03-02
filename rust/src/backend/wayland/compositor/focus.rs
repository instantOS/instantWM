use std::borrow::Cow;

use smithay::{
    backend::input::KeyState,
    desktop::{PopupKind, Window},
    input::{
        keyboard::{KeysymHandle, ModifiersState},
        Seat,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Serial},
    wayland::seat::WaylandFocus,
};

use super::WaylandState;

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
