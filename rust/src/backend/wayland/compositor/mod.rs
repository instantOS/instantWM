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

mod focus;
mod handlers;
mod state;

pub use focus::{KeyboardFocusTarget, PointerFocusTarget};
pub use state::{WaylandClientState, WaylandState, WindowIdMarker};

use smithay::{
    delegate_compositor, delegate_data_device, delegate_layer_shell, delegate_output, delegate_seat,
    delegate_shm, delegate_xdg_shell, delegate_xwayland_shell,
};

// ---------------------------------------------------------------------------
// Delegate macros — these MUST be at module level
// ---------------------------------------------------------------------------

delegate_compositor!(WaylandState);
delegate_data_device!(WaylandState);
delegate_shm!(WaylandState);
delegate_seat!(WaylandState);
delegate_xdg_shell!(WaylandState);
delegate_layer_shell!(WaylandState);
delegate_output!(WaylandState);
delegate_xwayland_shell!(WaylandState);
