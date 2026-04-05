use std::borrow::Cow;

use smithay::{
    backend::input::KeyState,
    desktop::{PopupKind, Window},
    input::{
        Seat,
        dnd::{DndFocus, Source},
        keyboard::{KeysymHandle, ModifiersState},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Serial},
    wayland::{seat::WaylandFocus, selection::data_device::WlOfferData},
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
    Window(Window),
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
            KeyboardFocusTarget::Window(w) => PointerFocusTarget::Window(w),
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
            PointerFocusTarget::Window(w) => w.alive(),
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
            PointerFocusTarget::Window(w) => w.wl_surface(),
            PointerFocusTarget::WlSurface(s) => Some(Cow::Borrowed(s)),
            PointerFocusTarget::Popup(p) => Some(Cow::Borrowed(p.wl_surface())),
        }
    }
}

impl PointerFocusTarget {
    fn with_surface<F>(&self, f: F)
    where
        F: FnOnce(&WlSurface),
    {
        if let Some(surface) = self.wl_surface() {
            f(surface.as_ref());
        } else {
            log::trace!("PointerFocusTarget has no wl_surface, dropping event");
        }
    }
}

impl DndFocus<WaylandState> for PointerFocusTarget {
    type OfferData<S: Source> = WlOfferData<S>;

    fn enter<S: Source>(
        &self,
        data: &mut WaylandState,
        dh: &smithay::reexports::wayland_server::DisplayHandle,
        source: std::sync::Arc<S>,
        seat: &Seat<WaylandState>,
        location: smithay::utils::Point<f64, smithay::utils::Logical>,
        serial: &Serial,
    ) -> Option<Self::OfferData<S>> {
        match self {
            PointerFocusTarget::Window(window) => window.wl_surface().and_then(|surface| {
                DndFocus::enter(surface.as_ref(), data, dh, source, seat, location, serial)
            }),
            PointerFocusTarget::WlSurface(surface) => {
                DndFocus::enter(surface, data, dh, source, seat, location, serial)
            }
            PointerFocusTarget::Popup(popup) => {
                DndFocus::enter(popup.wl_surface(), data, dh, source, seat, location, serial)
            }
        }
    }

    fn motion<S: Source>(
        &self,
        data: &mut WaylandState,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<WaylandState>,
        location: smithay::utils::Point<f64, smithay::utils::Logical>,
        time: u32,
    ) {
        match self {
            PointerFocusTarget::Window(window) => {
                if let Some(surface) = window.wl_surface() {
                    DndFocus::motion(surface.as_ref(), data, offer, seat, location, time);
                }
            }
            PointerFocusTarget::WlSurface(surface) => {
                DndFocus::motion(surface, data, offer, seat, location, time)
            }
            PointerFocusTarget::Popup(popup) => {
                DndFocus::motion(popup.wl_surface(), data, offer, seat, location, time)
            }
        }
    }

    fn leave<S: Source>(
        &self,
        data: &mut WaylandState,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<WaylandState>,
    ) {
        match self {
            PointerFocusTarget::Window(window) => {
                if let Some(surface) = window.wl_surface() {
                    DndFocus::leave(surface.as_ref(), data, offer, seat);
                }
            }
            PointerFocusTarget::WlSurface(surface) => DndFocus::leave(surface, data, offer, seat),
            PointerFocusTarget::Popup(popup) => {
                DndFocus::leave(popup.wl_surface(), data, offer, seat)
            }
        }
    }

    fn drop<S: Source>(
        &self,
        data: &mut WaylandState,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<WaylandState>,
    ) {
        match self {
            PointerFocusTarget::Window(window) => {
                if let Some(surface) = window.wl_surface() {
                    DndFocus::drop(surface.as_ref(), data, offer, seat);
                }
            }
            PointerFocusTarget::WlSurface(surface) => DndFocus::drop(surface, data, offer, seat),
            PointerFocusTarget::Popup(popup) => {
                DndFocus::drop(popup.wl_surface(), data, offer, seat)
            }
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
                        surface.as_ref(),
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
                        surface.as_ref(),
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
                        surface.as_ref(),
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
                        surface.as_ref(),
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
        self.with_surface(|surface| {
            smithay::input::pointer::PointerTarget::enter(surface, seat, data, event)
        });
    }

    fn motion(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::MotionEvent,
    ) {
        self.with_surface(|surface| {
            smithay::input::pointer::PointerTarget::motion(surface, seat, data, event)
        });
    }

    fn relative_motion(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::RelativeMotionEvent,
    ) {
        self.with_surface(|surface| {
            smithay::input::pointer::PointerTarget::relative_motion(surface, seat, data, event)
        });
    }

    fn button(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::ButtonEvent,
    ) {
        self.with_surface(|surface| {
            smithay::input::pointer::PointerTarget::button(surface, seat, data, event)
        });
    }

    fn axis(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        frame: smithay::input::pointer::AxisFrame,
    ) {
        self.with_surface(|surface| {
            smithay::input::pointer::PointerTarget::axis(surface, seat, data, frame)
        });
    }

    fn frame(&self, seat: &Seat<WaylandState>, data: &mut WaylandState) {
        self.with_surface(|surface| {
            smithay::input::pointer::PointerTarget::frame(surface, seat, data)
        });
    }

    fn gesture_swipe_begin(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GestureSwipeBeginEvent,
    ) {
        self.with_surface(|surface| {
            smithay::input::pointer::PointerTarget::gesture_swipe_begin(surface, seat, data, event)
        });
    }

    fn gesture_swipe_update(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GestureSwipeUpdateEvent,
    ) {
        self.with_surface(|surface| {
            smithay::input::pointer::PointerTarget::gesture_swipe_update(surface, seat, data, event)
        });
    }

    fn gesture_swipe_end(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GestureSwipeEndEvent,
    ) {
        self.with_surface(|surface| {
            smithay::input::pointer::PointerTarget::gesture_swipe_end(surface, seat, data, event)
        });
    }

    fn gesture_pinch_begin(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GesturePinchBeginEvent,
    ) {
        self.with_surface(|surface| {
            smithay::input::pointer::PointerTarget::gesture_pinch_begin(surface, seat, data, event)
        });
    }

    fn gesture_pinch_update(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GesturePinchUpdateEvent,
    ) {
        self.with_surface(|surface| {
            smithay::input::pointer::PointerTarget::gesture_pinch_update(surface, seat, data, event)
        });
    }

    fn gesture_pinch_end(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GesturePinchEndEvent,
    ) {
        self.with_surface(|surface| {
            smithay::input::pointer::PointerTarget::gesture_pinch_end(surface, seat, data, event)
        });
    }

    fn gesture_hold_begin(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GestureHoldBeginEvent,
    ) {
        self.with_surface(|surface| {
            smithay::input::pointer::PointerTarget::gesture_hold_begin(surface, seat, data, event)
        });
    }

    fn gesture_hold_end(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::pointer::GestureHoldEndEvent,
    ) {
        self.with_surface(|surface| {
            smithay::input::pointer::PointerTarget::gesture_hold_end(surface, seat, data, event)
        });
    }

    fn leave(&self, seat: &Seat<WaylandState>, data: &mut WaylandState, serial: Serial, time: u32) {
        self.with_surface(|surface| {
            smithay::input::pointer::PointerTarget::leave(surface, seat, data, serial, time)
        });
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
        self.with_surface(|surface| {
            smithay::input::touch::TouchTarget::down(surface, seat, data, event, seq)
        });
    }

    fn up(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::touch::UpEvent,
        seq: Serial,
    ) {
        self.with_surface(|surface| {
            smithay::input::touch::TouchTarget::up(surface, seat, data, event, seq)
        });
    }

    fn motion(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::touch::MotionEvent,
        seq: Serial,
    ) {
        self.with_surface(|surface| {
            smithay::input::touch::TouchTarget::motion(surface, seat, data, event, seq)
        });
    }

    fn frame(&self, seat: &Seat<WaylandState>, data: &mut WaylandState, seq: Serial) {
        self.with_surface(|surface| {
            smithay::input::touch::TouchTarget::frame(surface, seat, data, seq)
        });
    }

    fn cancel(&self, seat: &Seat<WaylandState>, data: &mut WaylandState, seq: Serial) {
        self.with_surface(|surface| {
            smithay::input::touch::TouchTarget::cancel(surface, seat, data, seq)
        });
    }

    fn shape(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::touch::ShapeEvent,
        seq: Serial,
    ) {
        self.with_surface(|surface| {
            smithay::input::touch::TouchTarget::shape(surface, seat, data, event, seq)
        });
    }

    fn orientation(
        &self,
        seat: &Seat<WaylandState>,
        data: &mut WaylandState,
        event: &smithay::input::touch::OrientationEvent,
        seq: Serial,
    ) {
        self.with_surface(|surface| {
            smithay::input::touch::TouchTarget::orientation(surface, seat, data, event, seq)
        });
    }
}
