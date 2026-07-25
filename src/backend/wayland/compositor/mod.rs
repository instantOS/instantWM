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
pub use state::{PendingLaunchContextMarker, WaylandClientState, WaylandState, WindowIdMarker};

use smithay::delegate_dispatch2;

// ---------------------------------------------------------------------------
// Dispatch delegation — this MUST be at module level
// ---------------------------------------------------------------------------

delegate_dispatch2!(WaylandState);
