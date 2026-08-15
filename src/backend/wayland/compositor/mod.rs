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
//! 3. **dispatch delegation** — `delegate_dispatch2!` generates the protocol
//!    `wayland_server::Dispatch` impls from those handler traits.
//!
//! The delegation macro MUST be called at module level (not inside an `impl`
//! block). It wires Smithay's internal message routing to the handler traits.

mod capture_common;
mod focus;
mod handlers;
pub(crate) mod image_capture;
pub(crate) mod layer_shell;
pub mod output;
pub mod protocols;
pub mod screencopy;
mod session_lock;
mod state;
pub mod window;
mod xdg_shell;
mod xwayland;

pub use focus::{KeyboardFocusTarget, PointerFocusTarget};
pub(crate) use state::PendingRenderTargets;
pub(crate) use state::TOUCH_POINTER_BUTTON_CODE;
pub use state::{PendingLaunchContextMarker, WaylandClientState, WaylandState, WindowIdMarker};

use smithay::delegate_dispatch2;

/// Construct the calloop event loop and Smithay compositor state shared by
/// production runtimes and compositor tests.
pub(crate) fn new_event_loop_and_state() -> (
    smithay::reexports::calloop::EventLoop<'static, WaylandState>,
    WaylandState,
) {
    let event_loop = smithay::reexports::calloop::EventLoop::try_new().expect("wayland event loop");
    let loop_handle = event_loop.handle();
    let display = smithay::reexports::wayland_server::Display::new().expect("wayland display");
    let state = WaylandState::new(display, &loop_handle);
    (event_loop, state)
}

// ---------------------------------------------------------------------------
// Dispatch delegation — this MUST be at module level
// ---------------------------------------------------------------------------

delegate_dispatch2!(WaylandState);
