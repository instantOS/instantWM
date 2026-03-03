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

pub mod compositor;

use crate::backend::{BackendKind, BackendOps};
use crate::types::{Rect, WindowId};

/// Wayland backend placeholder/state wrapper.
use std::cell::RefCell;
use std::ptr::NonNull;

use crate::backend::wayland::compositor::WaylandState;

#[derive(Default)]
pub struct WaylandBackend {
    state: RefCell<Option<NonNull<WaylandState>>>,
}

impl WaylandBackend {
    pub fn new() -> Self {
        Self {
            state: RefCell::new(None),
        }
    }

    pub fn attach_state(&self, state: &mut WaylandState) {
        *self.state.borrow_mut() = Some(NonNull::from(state));
    }

    pub fn close_window(&self, window: WindowId) -> bool {
        self.with_state(|state: &mut WaylandState| state.close_window(window))
            .unwrap_or(false)
    }

    pub fn window_title(&self, window: WindowId) -> Option<String> {
        self.with_state(|state: &mut WaylandState| state.window_title(window))
            .flatten()
    }

    fn with_state<T>(&self, f: impl FnOnce(&mut WaylandState) -> T) -> Option<T> {
        let mut ptr = *self.state.borrow();
        ptr.as_mut().map(|state| unsafe { f(state.as_mut()) })
    }
}

impl BackendOps for WaylandBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Wayland
    }

    fn resize_window(&self, window: WindowId, rect: Rect) {
        let _ = self.with_state(|state: &mut WaylandState| state.resize_window(window, rect));
    }

    fn raise_window(&self, window: WindowId) {
        let _ = self.with_state(|state: &mut WaylandState| state.raise_window(window));
    }

    fn restack(&self, windows: &[WindowId]) {
        let _ = self.with_state(|state: &mut WaylandState| state.restack(windows));
    }

    fn set_focus(&self, window: WindowId) {
        let _ = self.with_state(|state: &mut WaylandState| state.set_focus(window));
    }

    fn map_window(&self, window: WindowId) {
        let _ = self.with_state(|state: &mut WaylandState| state.map_window(window));
    }

    fn unmap_window(&self, window: WindowId) {
        let _ = self.with_state(|state: &mut WaylandState| state.unmap_window(window));
    }

    fn set_border_width(&self, _window: WindowId, _width: i32) {}

    fn window_exists(&self, window: WindowId) -> bool {
        self.with_state(|state: &mut WaylandState| state.window_exists(window))
            .unwrap_or(false)
    }

    fn flush(&self) {
        let _ = self.with_state(WaylandState::flush);
    }

    fn window_title(&self, window: WindowId) -> Option<String> {
        self.window_title(window)
    }
}
