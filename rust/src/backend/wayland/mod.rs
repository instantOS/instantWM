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

use crate::backend::BackendOps;
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

    pub fn xdisplay(&self) -> Option<u32> {
        self.with_state(|state: &mut WaylandState| state.xdisplay)
            .flatten()
    }

    /// Return the current pointer position in root (logical) coordinates,
    /// rounded to the nearest integer pixel.  Returns `None` if the Wayland
    /// state has not been attached yet.
    pub fn pointer_location(&self) -> Option<(i32, i32)> {
        self.with_state(|state: &mut WaylandState| {
            let loc = state.pointer.current_location();
            (loc.x.round() as i32, loc.y.round() as i32)
        })
    }

    /// Request the compositor to warp the hardware pointer to `(x, y)` in
    /// logical screen coordinates.  The warp is deferred to the next
    /// event-loop tick where the pointer handle and the external
    /// `pointer_location` variable are both updated together.
    pub fn warp_pointer(&self, x: f64, y: f64) {
        let _ = self.with_state(|state: &mut WaylandState| {
            state.request_warp(x, y);
        });
    }

    pub fn set_cursor_icon_override(&self, icon: Option<smithay::input::pointer::CursorIcon>) {
        let _ = self.with_state(|state: &mut WaylandState| {
            state.cursor_icon_override = icon;
            if icon.is_none() {
                state.cursor_image_status =
                    smithay::input::pointer::CursorImageStatus::default_named();
            }
        });
    }

    fn with_state<T>(&self, f: impl FnOnce(&mut WaylandState) -> T) -> Option<T> {
        let mut ptr = *self.state.borrow();
        ptr.as_mut().map(|state| unsafe { f(state.as_mut()) })
    }
}

impl BackendOps for WaylandBackend {
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
        self.with_state(|state: &mut WaylandState| state.window_title(window))
            .flatten()
    }

    fn set_keyboard_layout(
        &self,
        layout: &str,
        variant: &str,
        options: Option<&str>,
        model: Option<&str>,
    ) {
        let layout_str = layout.to_owned();
        let variant_str = variant.to_owned();
        let options_str = options.map(|s| s.to_owned());
        let model_str = model.map(|s| s.to_owned());
        let _ = self.with_state(move |state: &mut WaylandState| {
            state.set_keyboard_layout(
                &layout_str,
                &variant_str,
                options_str.as_deref(),
                model_str.as_deref(),
            );
        });
    }
}
