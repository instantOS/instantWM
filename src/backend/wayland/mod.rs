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
use crate::backend::BackendOutputInfo;
use crate::types::{Rect, WindowId};
use std::cell::RefCell;
use std::collections::HashMap;

use crate::backend::wayland::compositor::WaylandState;

/// Commands queued by WM logic, executed on WaylandState after WM returns.
#[derive(Debug, Clone)]
pub enum WmCommand {
    ResizeWindow(WindowId, Rect),
    RaiseWindow(WindowId),
    Restack(Vec<WindowId>),
    SetFocus(WindowId),
    MapWindow(WindowId),
    UnmapWindow(WindowId),
    Flush,
    WarpPointer(f64, f64),
    CloseWindow(WindowId),
    ClearKeyboardFocus,
    SetCursorIconOverride(Option<smithay::input::pointer::CursorIcon>),
    SetKeyboardLayout {
        layout: String,
        variant: String,
        options: Option<String>,
        model: Option<String>,
    },
    SetMonitorConfig {
        name: String,
        config: crate::config::config_toml::MonitorConfig,
    },
}

/// Wayland backend placeholder/state wrapper.
///
/// This struct acts as a bridge between the generic `Wm` logic and the
/// Smithay-specific `WaylandState`. Commands from WM logic are queued and
/// flushed after the WM returns, avoiding any need for raw pointers or
/// unsafe code in the backend bridge.
pub struct WaylandBackend {
    /// Pending operations from WM logic
    pending_ops: RefCell<Vec<WmCommand>>,
    /// Cached pointer location, updated by sync_cache each event loop tick
    cached_pointer_location: RefCell<Option<(i32, i32)>>,
    /// Cached xdisplay, updated by sync_cache
    cached_xdisplay: RefCell<Option<u32>>,
    /// Cached output info, updated by sync_cache
    cached_outputs: RefCell<Vec<BackendOutputInfo>>,
    /// Cached input device list, updated by sync_cache
    cached_input_devices: RefCell<Vec<String>>,
    /// Cached display names, updated by sync_cache
    cached_displays: RefCell<Vec<String>>,
    /// Cached display modes per display name, updated by sync_cache
    cached_display_modes: RefCell<HashMap<String, Vec<String>>>,
    /// Cached keyboard focus window, updated by sync_cache
    cached_keyboard_focus: RefCell<Option<WindowId>>,
    /// True when a layer-shell surface wants keyboard focus (e.g. fuzzel/rofi/dmenu)
    cached_has_layer_focus: RefCell<bool>,
}

impl WaylandBackend {
    pub fn new() -> Self {
        Self {
            pending_ops: RefCell::new(Vec::new()),
            cached_pointer_location: RefCell::new(None),
            cached_xdisplay: RefCell::new(None),
            cached_outputs: RefCell::new(Vec::new()),
            cached_input_devices: RefCell::new(Vec::new()),
            cached_displays: RefCell::new(Vec::new()),
            cached_display_modes: RefCell::new(HashMap::new()),
            cached_keyboard_focus: RefCell::new(None),
            cached_has_layer_focus: RefCell::new(false),
        }
    }

    /// Drain all pending commands. Called from the event loop after WM logic.
    pub fn drain_ops(&self) -> Vec<WmCommand> {
        std::mem::take(&mut *self.pending_ops.borrow_mut())
    }

    /// Update cached values from WaylandState. Called after WM logic completes.
    pub fn sync_cache(&self, state: &WaylandState) {
        let loc = state.pointer.current_location();
        *self.cached_pointer_location.borrow_mut() =
            Some((loc.x.round() as i32, loc.y.round() as i32));
        *self.cached_xdisplay.borrow_mut() = state.xdisplay;
        *self.cached_keyboard_focus.borrow_mut() = state
            .keyboard
            .current_focus()
            .and_then(|focus| state.keyboard_focus_to_window_id(&focus));
        *self.cached_has_layer_focus.borrow_mut() = state.has_layer_keyboard_focus();

        // Update cached outputs
        let outputs: Vec<_> = state
            .space
            .outputs()
            .map(|o: &smithay::output::Output| BackendOutputInfo {
                name: o.name(),
                rect: {
                    let geom = state.space.output_geometry(o).unwrap_or_default();
                    Rect {
                        x: geom.loc.x,
                        y: geom.loc.y,
                        w: geom.size.w,
                        h: geom.size.h,
                    }
                },
            })
            .collect();
        *self.cached_outputs.borrow_mut() = outputs;

        // Update cached input devices
        let devices: Vec<_> = state
            .tracked_devices
            .iter()
            .map(|d: &smithay::reexports::input::Device| {
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
            .collect();
        *self.cached_input_devices.borrow_mut() = devices;

        // Update cached display names and modes
        let displays = state.list_displays();
        let mut display_modes = HashMap::new();
        for display in &displays {
            display_modes.insert(display.clone(), state.list_display_modes(display));
        }
        *self.cached_displays.borrow_mut() = displays;
        *self.cached_display_modes.borrow_mut() = display_modes;
    }

    // -- Public query methods (use cached values) --

    /// List all connected display names (cached).
    pub fn list_displays(&self) -> Vec<String> {
        self.cached_displays.borrow().clone()
    }

    /// List available display modes for a display (cached).
    pub fn list_display_modes(&self, display: &str) -> Vec<String> {
        self.cached_display_modes
            .borrow()
            .get(display)
            .cloned()
            .unwrap_or_default()
    }

    pub fn xdisplay(&self) -> Option<u32> {
        *self.cached_xdisplay.borrow()
    }

    pub fn pointer_location(&self) -> Option<(i32, i32)> {
        *self.cached_pointer_location.borrow()
    }

    pub fn window_title(&self, _window: WindowId) -> Option<String> {
        // Wayland titles are pushed to WM via compositor events, not pulled.
        None
    }

    pub fn is_keyboard_focused_on(&self, window: WindowId) -> bool {
        self.cached_keyboard_focus
            .borrow()
            .is_some_and(|focus| focus == window)
    }

    /// Returns `true` when a layer-shell surface (fuzzel/rofi/dmenu) wants keyboard focus.
    pub fn has_layer_keyboard_focus(&self) -> bool {
        *self.cached_has_layer_focus.borrow()
    }

    pub fn close_window(&self, window: WindowId) -> bool {
        self.pending_ops
            .borrow_mut()
            .push(WmCommand::CloseWindow(window));
        true // optimistic — actual close happens during flush
    }

    pub fn clear_keyboard_focus(&self) {
        self.pending_ops
            .borrow_mut()
            .push(WmCommand::ClearKeyboardFocus);
    }

    pub fn set_cursor_icon_override(&self, icon: Option<smithay::input::pointer::CursorIcon>) {
        self.pending_ops
            .borrow_mut()
            .push(WmCommand::SetCursorIconOverride(icon));
    }

    pub fn warp_pointer(&self, x: f64, y: f64) {
        self.pending_ops
            .borrow_mut()
            .push(WmCommand::WarpPointer(x, y));
    }
}

impl Default for WaylandBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl BackendOps for WaylandBackend {
    fn resize_window(&self, window: WindowId, rect: Rect) {
        self.pending_ops
            .borrow_mut()
            .push(WmCommand::ResizeWindow(window, rect));
    }

    fn raise_window(&self, window: WindowId) {
        self.pending_ops
            .borrow_mut()
            .push(WmCommand::RaiseWindow(window));
    }

    fn restack(&self, windows: &[WindowId]) {
        self.pending_ops
            .borrow_mut()
            .push(WmCommand::Restack(windows.to_vec()));
    }

    fn set_focus(&self, window: WindowId) {
        self.pending_ops
            .borrow_mut()
            .push(WmCommand::SetFocus(window));
    }

    fn map_window(&self, window: WindowId) {
        self.pending_ops
            .borrow_mut()
            .push(WmCommand::MapWindow(window));
    }

    fn unmap_window(&self, window: WindowId) {
        self.pending_ops
            .borrow_mut()
            .push(WmCommand::UnmapWindow(window));
    }

    fn window_exists(&self, _window: WindowId) -> bool {
        // Cannot query WaylandState here. The WM tracks window existence
        // through its client list; destruction events remove windows reactively.
        true
    }

    fn flush(&self) {
        self.pending_ops.borrow_mut().push(WmCommand::Flush);
    }

    fn pointer_location(&self) -> Option<(i32, i32)> {
        *self.cached_pointer_location.borrow()
    }

    fn warp_pointer(&self, x: f64, y: f64) {
        self.pending_ops
            .borrow_mut()
            .push(WmCommand::WarpPointer(x, y));
    }

    fn window_title(&self, _window: WindowId) -> Option<String> {
        // Titles are pushed to WM via compositor events, not pulled.
        None
    }

    fn set_keyboard_layout(
        &self,
        layout: &str,
        variant: &str,
        options: Option<&str>,
        model: Option<&str>,
    ) {
        self.pending_ops
            .borrow_mut()
            .push(WmCommand::SetKeyboardLayout {
                layout: layout.to_owned(),
                variant: variant.to_owned(),
                options: options.map(|s| s.to_owned()),
                model: model.map(|s| s.to_owned()),
            });
    }

    fn set_monitor_config(&self, name: &str, config: &crate::config::config_toml::MonitorConfig) {
        self.pending_ops
            .borrow_mut()
            .push(WmCommand::SetMonitorConfig {
                name: name.to_owned(),
                config: config.clone(),
            });
    }

    fn get_outputs(&self) -> Vec<crate::backend::BackendOutputInfo> {
        self.cached_outputs.borrow().clone()
    }

    fn get_input_devices(&self) -> Vec<String> {
        self.cached_input_devices.borrow().clone()
    }
}
