//! Wayland compositor backend using Smithay.
//!
//! This module implements the Wayland side of instantWM's dual-backend
//! architecture.  When compiled with `--features wayland_backend`, instantWM
//! can run as a standalone Wayland compositor (with XWayland support for
//! legacy X11 clients).
//!
//! # Architecture
//!
//! Smithay is a *library*, not a framework.  Our compositor state struct
//! (`WaylandState`) stores all the Smithay protocol state objects and
//! implements the corresponding handler traits.  The calloop event loop
//! drives everything:
//!
//! ```text
//! calloop EventLoop
//!  ├─ ListeningSocketSource   → accept new Wayland clients
//!  ├─ Generic(Display)        → dispatch protocol messages
//!  ├─ XWayland source         → spawn / manage XWayland
//!  └─ (future) backend source → DRM/udev or nested winit
//! ```
//!
//! # Smithay Quick Reference (for future implementors)
//!
//! ## Adding a new Wayland protocol
//!
//! 1. Add a `FooState` field to `WaylandState`.
//! 2. Initialise it in `WaylandState::new()` with `FooState::new::<WaylandState>(&dh)`.
//! 3. Implement the `FooHandler` trait on `WaylandState`.
//! 4. Call `smithay::delegate_foo!(WaylandState);` at module level.
//!
//! ## Focus dispatch
//!
//! Smithay's `SeatHandler` uses associated types (`KeyboardFocus`,
//! `PointerFocus`) to determine what can receive input.  Our focus target
//! enums (defined below) cover both native Wayland surfaces and XWayland
//! X11 surfaces so input routing is polymorphic.
//!
//! ## XWayland
//!
//! XWayland is started asynchronously.  `XWayland::spawn()` returns a
//! calloop source; when `XWaylandEvent::Ready` fires we create an `X11Wm`
//! and store it in `WaylandState::xwm`.  The `XwmHandler` trait bridges
//! X11 window events into our WM logic.
//!
//! ## Rendering
//!
//! Rendering is NOT handled here.  A future `renderer` module will abstract
//! over software (pixman) and hardware (GL/Vulkan) renderers.  The bar will
//! need a Wayland-native renderer path (layer-shell surface or custom
//! rendering).

#[cfg(feature = "wayland_backend")]
pub mod compositor;

use crate::backend::{BackendKind, BackendOps};
use crate::types::{Rect, WindowId};

/// Wayland backend placeholder/state wrapper.
#[derive(Default)]
pub struct WaylandBackend;

impl WaylandBackend {
    pub fn new() -> Self {
        Self
    }
}

impl BackendOps for WaylandBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Wayland
    }

    fn resize_window(&self, _window: WindowId, _rect: Rect) {}

    fn raise_window(&self, _window: WindowId) {}

    fn restack(&self, _windows: &[WindowId]) {}

    fn set_focus(&self, _window: WindowId) {}

    fn map_window(&self, _window: WindowId) {}

    fn unmap_window(&self, _window: WindowId) {}

    fn flush(&self) {}
}
