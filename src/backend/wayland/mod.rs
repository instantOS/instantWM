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

use crate::backend::{BackendOps, WindowProtocol};
use crate::types::{Rect, WindowId};

/// Wayland backend placeholder/state wrapper.
///
/// This struct acts as a bridge between the generic `Wm` logic and the
/// Smithay-specific `WaylandState`. Since `WaylandState` is owned by the
/// event loop (calloop), and the `Wm` struct (which owns this backend)
/// is passed into the event loop's callback, we use an `Option<NonNull>`
/// pointer to establish a safe-at-runtime circular reference.
///
/// This design avoids the overhead of `Rc<RefCell<...>>` cycles while
/// maintaining the ability for the WM to perform backend-specific actions.
use std::cell::RefCell;
use std::ptr::NonNull;

use crate::backend::wayland::compositor::WaylandState;

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

    /// List available display modes for a display (format: "WIDTHxHEIGHT@REFRESH").
    pub fn list_display_modes(&self, display: &str) -> Vec<String> {
        self.with_state(|state: &mut WaylandState| state.list_display_modes(display))
            .unwrap_or_default()
    }

    /// List all connected display names.
    pub fn list_displays(&self) -> Vec<String> {
        self.with_state(|state: &mut WaylandState| state.list_displays())
            .unwrap_or_default()
    }

    pub fn close_window(&self, window: WindowId) -> bool {
        self.with_state(|state: &mut WaylandState| state.close_window(window))
            .unwrap_or(false)
    }

    pub fn window_title(&self, window: WindowId) -> Option<String> {
        self.with_state(|state: &mut WaylandState| state.window_title(window))
            .flatten()
    }

    pub fn window_protocol(&self, window: WindowId) -> WindowProtocol {
        self.with_state(|state: &mut WaylandState| state.window_protocol(window))
            .unwrap_or(WindowProtocol::Unknown)
    }

    pub fn xdisplay(&self) -> Option<u32> {
        self.with_state(|state: &mut WaylandState| state.xdisplay)
            .flatten()
    }

    pub fn pointer_location(&self) -> Option<(i32, i32)> {
        self.with_state(|state: &mut WaylandState| {
            let loc = state.pointer.current_location();
            (loc.x.round() as i32, loc.y.round() as i32)
        })
    }

    pub fn warp_pointer(&self, x: f64, y: f64) {
        let _ = self.with_state(|state: &mut WaylandState| {
            state.request_warp(x, y);
        });
    }

    pub fn request_bar_redraw(&self) -> bool {
        self.with_state(|state: &mut WaylandState| state.request_bar_redraw())
            .is_some()
    }

    pub fn request_space_sync(&self) {
        let _ = self.with_state(|state: &mut WaylandState| state.request_space_sync());
    }

    pub fn is_keyboard_focused_on(&self, window: WindowId) -> bool {
        self.with_state(|state: &mut WaylandState| state.is_seat_focused_on(window))
            .unwrap_or(false)
    }

    pub fn clear_keyboard_focus(&self) {
        let _ = self.with_state(|state: &mut WaylandState| state.clear_seat_focus());
    }

    pub fn set_cursor_icon_override(&self, icon: Option<smithay::input::pointer::CursorIcon>) {
        let _ = self.with_state(|state: &mut WaylandState| {
            state.cursor_icon_override = icon;
        });
    }

    pub(crate) fn with_state<T>(&self, f: impl FnOnce(&mut WaylandState) -> T) -> Option<T> {
        let maybe_ptr = *self.state.borrow();
        maybe_ptr.map(|mut ptr| unsafe { f(ptr.as_mut()) })
    }
}

impl Default for WaylandBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl BackendOps for WaylandBackend {
    fn resize_window(&self, window: WindowId, rect: Rect) {
        let _ = self.with_state(|state: &mut WaylandState| state.resize_window(window, rect));
    }

    fn raise_window_visual_only(&self, window: WindowId) {
        let _ = self.with_state(|state: &mut WaylandState| state.raise_window_visual_only(window));
    }

    fn apply_z_order(&self, windows: &[WindowId]) {
        let _ = self.with_state(|state: &mut WaylandState| state.apply_z_order(windows));
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

    fn window_exists(&self, window: WindowId) -> bool {
        self.with_state(|state: &mut WaylandState| state.window_exists(window))
            .unwrap_or(false)
    }

    fn flush(&self) {
        let _ = self.with_state(WaylandState::flush);
    }

    fn pointer_location(&self) -> Option<(i32, i32)> {
        self.with_state(|state: &mut WaylandState| {
            let loc = state.pointer.current_location();
            (loc.x.round() as i32, loc.y.round() as i32)
        })
    }

    fn warp_pointer(&self, x: f64, y: f64) {
        let _ = self.with_state(|state: &mut WaylandState| {
            state.request_warp(x, y);
        });
    }

    fn window_title(&self, window: WindowId) -> Option<String> {
        self.with_state(|state: &mut WaylandState| state.window_title(window))
            .flatten()
    }

    fn window_protocol(&self, window: WindowId) -> WindowProtocol {
        self.window_protocol(window)
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

    fn set_monitor_config(&self, name: &str, config: &crate::config::config_toml::MonitorConfig) {
        let name_str = name.to_owned();
        let config_clone = config.clone();
        let _ = self.with_state(move |state: &mut WaylandState| {
            state.set_output_config(&name_str, &config_clone);
        });
    }

    fn get_outputs(&self) -> Vec<crate::backend::BackendOutputInfo> {
        self.with_state(|state: &mut WaylandState| {
            state
                .space
                .outputs()
                .map(|o| crate::backend::BackendOutputInfo {
                    name: o.name(),
                    rect: {
                        let geom = state.space.output_geometry(o).unwrap_or_default();
                        crate::types::Rect {
                            x: geom.loc.x,
                            y: geom.loc.y,
                            w: geom.size.w,
                            h: geom.size.h,
                        }
                    },
                    scale: o.current_scale().fractional_scale(),
                    vrr_support: state
                        .output_vrr_metadata(&o.name())
                        .map(|m| m.vrr_support)
                        .unwrap_or(crate::backend::BackendVrrSupport::Unsupported),
                    vrr_mode: state.output_vrr_metadata(&o.name()).map(|m| m.vrr_mode),
                    vrr_enabled: state
                        .output_vrr_metadata(&o.name())
                        .is_some_and(|m| m.vrr_enabled),
                })
                .collect()
        })
        .unwrap_or_default()
    }

    fn get_input_devices(&self) -> Vec<String> {
        self.with_state(|state: &mut WaylandState| {
            state
                .runtime
                .tracked_devices
                .iter()
                .map(|d| {
                    use smithay::reexports::input::DeviceCapability;
                    let mut caps = Vec::new();
                    if d.has_capability(DeviceCapability::Keyboard) {
                        caps.push("keyboard");
                    }
                    if d.has_capability(DeviceCapability::Pointer) {
                        caps.push("pointer");
                    }
                    if d.has_capability(DeviceCapability::Touch) {
                        caps.push("touch");
                    }
                    if d.has_capability(DeviceCapability::TabletTool) {
                        caps.push("tablet_tool");
                    }
                    if d.has_capability(DeviceCapability::TabletPad) {
                        caps.push("tablet_pad");
                    }
                    if d.has_capability(DeviceCapability::Gesture) {
                        caps.push("gesture");
                    }
                    if d.has_capability(DeviceCapability::Switch) {
                        caps.push("switch");
                    }
                    format!("{} (capabilities: {})", d.name(), caps.join(", "))
                })
                .collect()
        })
        .unwrap_or_default()
    }
}
