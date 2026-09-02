//! Wayland compositor backend using Smithay.
//!
//! This module implements the Wayland side of instantWM's dual-backend
//! architecture. It provides nested and standalone DRM/KMS compositor modes,
//! with XWayland support for legacy X11 clients. Both modes ship alongside
//! the X11 window manager and are selected at runtime.
//!
//! # Architecture
//!
//! Smithay is a *library*, not a framework. The backend is divided by
//! responsibility:
//!
//! - [`compositor`] owns Smithay protocol state and handler implementations.
//! - [`input`] translates winit/libinput events into compositor and WM input.
//! - [`render`] builds scenes and submits nested or DRM frames.
//! - [`runtime`] owns startup, queued-command dispatch, and event loops.
//! - [`session`] owns the socket, environment, systemd, and XWayland lifecycle.
//! - [`bootstrap`] initializes backend-neutral WM state for Wayland.
//!
//! The calloop event loop drives everything:
//!
//! ```text
//! calloop EventLoop
//!  ├─ ListeningSocketSource   → accept new Wayland clients
//!  ├─ Generic(Display)        → dispatch protocol messages
//!  ├─ XWayland source         → spawn / manage XWayland
//!  └─ backend sources         → DRM/udev/libinput or nested winit
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
//! [`render`] shares cursor, scene, and frame-callback policy while keeping
//! nested-winit and DRM/KMS submission mechanics separate.

pub mod bar;
pub mod bootstrap;
pub mod commands;
pub mod compositor;
pub mod init;
pub mod input;
pub(crate) mod output;
pub mod render;
pub mod runtime;
pub mod session;
pub mod visibility;

use crate::backend::{OutputOps, PointerOps, WindowOps, WindowProtocol};
use crate::types::{Point, Rect, WindowId};

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

    pub fn pointer_location(&self) -> Option<Point> {
        self.with_state(|state: &mut WaylandState| {
            let loc = state.pointer.current_location();
            Point::from_f64_round(loc.x, loc.y)
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

    pub fn request_render(&self) {
        let _ = self.with_state(|state: &mut WaylandState| state.request_render());
    }

    pub fn is_keyboard_focused_on(&self, window: WindowId) -> bool {
        self.with_state(|state: &mut WaylandState| state.is_seat_focused_on(window))
            .unwrap_or(false)
    }

    pub fn clear_keyboard_focus(&self) {
        let _ = self.with_state(|state: &mut WaylandState| state.clear_seat_focus());
    }

    /// Project the core focus transaction into the surface's activated state.
    pub(crate) fn set_window_activated(&self, window: WindowId, activated: bool) {
        let _ = self
            .with_state(|state: &mut WaylandState| state.set_window_activated(window, activated));
    }

    pub fn set_cursor_icon_override(&self, icon: Option<smithay::input::pointer::CursorIcon>) {
        let _ = self.with_state(|state: &mut WaylandState| {
            if state.cursor_icon_override == icon {
                return;
            }
            state.cursor_icon_override = icon;
            state.request_render();
        });
    }

    /// Apply the compositor-native keyboard layout. X11 uses `setxkbmap`
    /// directly and deliberately does not pretend to provide this capability.
    pub fn set_keyboard_layout(
        &self,
        layout: &str,
        variant: &str,
        options: Option<&str>,
        model: Option<&str>,
    ) {
        let layout = layout.to_owned();
        let variant = variant.to_owned();
        let options = options.map(str::to_owned);
        let model = model.map(str::to_owned);
        let _ = self.with_state(move |state| {
            state.set_keyboard_layout(&layout, &variant, options.as_deref(), model.as_deref());
        });
    }

    /// Return Wayland input devices. This is intentionally not part of the
    /// cross-backend window capability trait.
    pub fn get_input_devices(&self) -> Vec<String> {
        self.with_state(|state: &mut WaylandState| {
            state
                .runtime
                .tracked_devices
                .iter()
                .map(|d| {
                    use smithay::backend::input::Device as InputDevice;
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
                    format!(
                        "{}: {} (capabilities: {})",
                        InputDevice::id(d),
                        d.name(),
                        caps.join(", ")
                    )
                })
                .collect()
        })
        .unwrap_or_default()
    }

    pub(crate) fn with_state<T>(&self, f: impl FnOnce(&mut WaylandState) -> T) -> Option<T> {
        let maybe_ptr = *self.state.borrow();
        maybe_ptr.map(|mut ptr| unsafe { f(ptr.as_mut()) })
    }

    pub(crate) fn sync_window_presentation(&self, window: WindowId) {
        let _ = self.with_state(|state| state.sync_window_presentation(window));
    }

    pub(crate) fn take_current_window_animation_rect(
        &self,
        win: WindowId,
        now: std::time::Instant,
    ) -> Option<Rect> {
        self.with_state(|state| state.take_current_window_animation_rect(win, now))
            .flatten()
    }

    pub(crate) fn cancel_window_animation(&self, win: WindowId) {
        let _ = self.with_state(|state| state.drop_window_animation(win));
    }

    pub(crate) fn window_animation_targets(&self, win: WindowId, target: Rect) -> bool {
        self.with_state(|state| state.animation_targets_outer_rect(win, target))
            .unwrap_or(false)
    }

    pub(crate) fn begin_window_animation(
        &self,
        win: WindowId,
        from: Rect,
        to: Rect,
        duration: std::time::Duration,
    ) {
        let _ = self.with_state(|state| {
            state.set_window_target_rect(
                win,
                to,
                crate::backend::wayland::compositor::window::animations::WindowMoveMode::AnimateFrom {
                    from,
                    duration,
                },
            );
        });
    }

    pub(crate) fn prepare_launch_environment(
        &self,
        command: &mut std::process::Command,
        selected_window: Option<WindowId>,
        context: crate::client::LaunchContext,
    ) {
        use smithay::wayland::seat::WaylandFocus;

        if let Some(token) = self.with_state(|state| {
            let source_surface = selected_window.and_then(|win| {
                state
                    .find_window(win)
                    .and_then(|window| window.wl_surface().map(|surface| surface.into_owned()))
            });
            let token_data = smithay::wayland::xdg_activation::XdgActivationTokenData {
                surface: source_surface,
                ..Default::default()
            };
            let _ = token_data
                .user_data
                .insert_if_missing_threadsafe(|| context);
            let (token, _) = state
                .xdg_activation_state
                .create_external_token(Some(token_data));
            token.as_str().to_owned()
        }) {
            command.env("XDG_ACTIVATION_TOKEN", token);
        }

        if let Some(display) = self.xdisplay() {
            command.env("DISPLAY", format!(":{display}"));
        } else if let Ok(display) = std::env::var("DISPLAY") {
            command.env("DISPLAY", display);
        }
    }
}

impl Default for WaylandBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowOps for WaylandBackend {
    fn resize_window(&self, window: WindowId, rect: Rect) {
        let _ = self.with_state(|state: &mut WaylandState| state.resize_window(window, rect));
    }

    fn set_border_width(&self, _window: WindowId, _width: i32) {
        // Wayland borders are compositor-rendered from core client state.
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
        let _ = self.with_state(|state: &mut WaylandState| state.map_window_in_space(window));
    }

    fn unmap_window(&self, window: WindowId) {
        let _ = self.with_state(|state: &mut WaylandState| state.unmap_window_from_space(window));
    }

    fn window_exists(&self, window: WindowId) -> bool {
        self.with_state(|state: &mut WaylandState| state.window_exists(window))
            .unwrap_or(false)
    }

    fn window_animation_active(&self, window: WindowId) -> bool {
        self.with_state(|state: &mut WaylandState| state.window_has_active_animation(window))
            .unwrap_or(false)
    }

    fn flush(&self) {
        let _ = self.with_state(WaylandState::flush);
    }

    fn window_protocol(&self, window: WindowId) -> WindowProtocol {
        WaylandBackend::window_protocol(self, window)
    }
}

impl PointerOps for WaylandBackend {
    fn pointer_location(&self) -> Option<Point> {
        WaylandBackend::pointer_location(self)
    }

    fn warp_pointer(&self, x: f64, y: f64) {
        WaylandBackend::warp_pointer(self, x, y);
    }
}

/// Map the WM's cursor presentation onto Wayland cursor icons.
///
/// `None` means "no override" — the default cursor applies. This projection
/// lives with the backend because it exists only for compositor-rendered
/// cursors; X11 uses server cursor fonts instead (`AltCursor::to_x11_index`).
fn wayland_cursor_icon(
    style: crate::types::AltCursor,
) -> Option<smithay::input::pointer::CursorIcon> {
    use smithay::input::pointer::CursorIcon;
    match style {
        crate::types::AltCursor::Default => None,
        crate::types::AltCursor::Move => Some(CursorIcon::Grabbing),
        crate::types::AltCursor::VerticalAdjust => Some(CursorIcon::NsResize),
        crate::types::AltCursor::HorizontalAdjust => Some(CursorIcon::EwResize),
        crate::types::AltCursor::Close => Some(CursorIcon::NotAllowed),
        crate::types::AltCursor::Resize(dir) => Some(match dir {
            crate::types::ResizeDirection::TopLeft => CursorIcon::NwResize,
            crate::types::ResizeDirection::Top => CursorIcon::NResize,
            crate::types::ResizeDirection::TopRight => CursorIcon::NeResize,
            crate::types::ResizeDirection::Right => CursorIcon::EResize,
            crate::types::ResizeDirection::BottomRight => CursorIcon::SeResize,
            crate::types::ResizeDirection::Bottom => CursorIcon::SResize,
            crate::types::ResizeDirection::BottomLeft => CursorIcon::SwResize,
            crate::types::ResizeDirection::Left => CursorIcon::WResize,
        }),
    }
}

impl crate::backend::InteractionProjectionOps for crate::contexts::WmCtxWayland<'_> {
    fn reconcile_interaction_projection(
        &mut self,
        desired: crate::core_state::InteractionProjection,
    ) {
        self.wayland
            .set_cursor_icon_override(wayland_cursor_icon(desired.cursor));
        let _ = self
            .wayland
            .with_state(|state| state.reconcile_interactive_resize(desired.active_resize_window));
    }
}

impl crate::backend::WindowCloseOps for crate::contexts::WmCtxWayland<'_> {
    fn close_window(&mut self, window: WindowId) {
        let _ = self.wayland.close_window(window);
    }
}

impl OutputOps for WaylandBackend {
    fn apply_monitor_configs(
        &self,
        configs: &std::collections::HashMap<String, crate::config::config_toml::MonitorConfig>,
    ) {
        let configs = configs.clone();
        let _ = self.with_state(move |state: &mut WaylandState| {
            let output_names: Vec<_> = state
                .output_management_state
                .outputs()
                .iter()
                .map(|output| output.name())
                .collect();
            for name in output_names {
                if let Some(config) = configs.get(&name).or_else(|| configs.get("*")) {
                    state.set_output_config(&name, config);
                }
            }
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
}

impl crate::backend::LayoutInteractionOps for crate::contexts::WmCtxWayland<'_> {
    fn begin_modal_keyboard(&mut self) -> bool {
        true
    }

    fn end_modal_keyboard(&mut self) {}

    fn layout_preview_changed(
        &mut self,
        rect: Option<Rect>,
        style: crate::types::InteractionOutlineStyle,
        target: Option<crate::types::WindowId>,
        animate: bool,
        duration: std::time::Duration,
    ) {
        let _ = self.wayland.with_state(|state| {
            state.set_layout_preview_target(rect, style, target, animate, duration)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{WaylandBackend, wayland_cursor_icon};
    use crate::backend::{WindowOps, WindowProtocol};
    use crate::types::{AltCursor, ResizeDirection, WindowId};
    use smithay::input::pointer::CursorIcon;

    #[test]
    fn window_protocol_trait_dispatch_delegates_to_inherent_query() {
        let backend = WaylandBackend::new();
        let ops: &dyn WindowOps = &backend;

        assert_eq!(ops.window_protocol(WindowId(1)), WindowProtocol::Unknown);
    }

    #[test]
    fn cursor_projection_covers_shared_resize_directions() {
        assert_eq!(wayland_cursor_icon(AltCursor::Default), None);
        assert_eq!(
            wayland_cursor_icon(AltCursor::Move),
            Some(CursorIcon::Grabbing)
        );
        assert_eq!(
            wayland_cursor_icon(AltCursor::VerticalAdjust),
            Some(CursorIcon::NsResize)
        );
        assert_eq!(
            wayland_cursor_icon(AltCursor::HorizontalAdjust),
            Some(CursorIcon::EwResize)
        );
        assert_eq!(
            wayland_cursor_icon(AltCursor::Close),
            Some(CursorIcon::NotAllowed)
        );

        for (direction, expected) in [
            (ResizeDirection::TopLeft, CursorIcon::NwResize),
            (ResizeDirection::Top, CursorIcon::NResize),
            (ResizeDirection::TopRight, CursorIcon::NeResize),
            (ResizeDirection::Right, CursorIcon::EResize),
            (ResizeDirection::BottomRight, CursorIcon::SeResize),
            (ResizeDirection::Bottom, CursorIcon::SResize),
            (ResizeDirection::BottomLeft, CursorIcon::SwResize),
            (ResizeDirection::Left, CursorIcon::WResize),
        ] {
            assert_eq!(
                wayland_cursor_icon(AltCursor::Resize(direction)),
                Some(expected)
            );
        }
    }
}
