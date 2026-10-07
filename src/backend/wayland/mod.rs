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

use crate::backend::wayland::compositor::WaylandState;

/// Native capabilities borrowed exclusively for one shared WM operation.
/// Queries borrow immutably; effects require `&mut self`. The borrow checker
/// enforces lifetime, stable address and exclusive/reentrant access without a
/// RefCell, mutex, stored address, or callback-phase convention.
///
/// `tests/borrow_contract.py` checks these contracts against the actual API,
/// including rejected external state access and rejected reentrant effects.
pub struct WaylandBackend<'a> {
    pub(crate) state: &'a mut WaylandState,
}

impl<'a> WaylandBackend<'a> {
    pub fn new(state: &'a mut WaylandState) -> Self {
        Self { state }
    }

    pub(crate) fn reborrow(&mut self) -> WaylandBackend<'_> {
        WaylandBackend::new(self.state)
    }

    pub fn close_window(&mut self, window: WindowId) -> bool {
        self.with_state(|state: &mut WaylandState| state.close_window(window))
    }

    pub fn window_title(&self, window: WindowId) -> Option<String> {
        self.with_state_ref(|state: &WaylandState| state.native.window_title(window))
    }

    pub fn window_protocol(&self, window: WindowId) -> WindowProtocol {
        self.with_state_ref(|state: &WaylandState| state.native.window_protocol(window))
    }

    pub fn xdisplay(&self) -> Option<u32> {
        self.with_state_ref(|state: &WaylandState| state.native.xdisplay)
    }

    pub fn pointer_location(&self) -> Option<Point> {
        Some(self.with_state_ref(|state: &WaylandState| {
            let loc = state.native.pointer.current_location();
            Point::from_f64_round(loc.x, loc.y)
        }))
    }

    pub fn warp_pointer(&mut self, x: f64, y: f64) {
        self.with_state(|state: &mut WaylandState| {
            state.native.request_warp(x, y);
        });
    }

    pub fn request_space_sync(&mut self) {
        self.with_state(|state: &mut WaylandState| state.native.request_space_sync());
    }

    pub fn request_render(&mut self) {
        self.with_state(|state: &mut WaylandState| state.native.request_render());
    }

    pub fn set_cursor_icon_override(&mut self, icon: Option<smithay::input::pointer::CursorIcon>) {
        self.with_state(|state: &mut WaylandState| {
            if state.native.cursor_icon_override == icon {
                return;
            }
            state.native.cursor_icon_override = icon;
            state.native.request_render();
        });
    }

    /// Apply the compositor-native keyboard layout. X11 uses `setxkbmap`
    /// directly and deliberately does not pretend to provide this capability.
    pub fn set_keyboard_layout(
        &mut self,
        layout: &str,
        variant: &str,
        options: Option<&str>,
        model: Option<&str>,
    ) -> Result<(), String> {
        self.with_state(|state| state.set_keyboard_layout(layout, variant, options, model))
    }

    /// Return Wayland input devices. This is intentionally not part of the
    /// cross-backend window capability trait.
    pub fn get_input_devices(&self) -> Vec<String> {
        self.with_state_ref(|state: &WaylandState| {
            state
                .native
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
    }

    pub(crate) fn with_state_ref<T>(&self, f: impl FnOnce(&WaylandState) -> T) -> T {
        f(self.state)
    }

    pub(crate) fn with_state<T>(&mut self, f: impl FnOnce(&mut WaylandState) -> T) -> T {
        f(self.state)
    }

    pub(crate) fn sync_window_presentation(&mut self, window: WindowId) {
        self.with_state(|state| {
            state
                .native
                .sync_window_presentation(&state.wm.core.state, window)
        });
    }

    pub(crate) fn take_current_window_animation_rect(
        &mut self,
        win: WindowId,
        now: std::time::Instant,
    ) -> Option<Rect> {
        self.with_state(|state| state.native.take_current_window_animation_rect(win, now))
    }

    pub(crate) fn cancel_window_animation(&mut self, win: WindowId) {
        self.with_state(|state| state.native.drop_window_animation(win));
    }

    pub(crate) fn window_animation_targets(&self, win: WindowId, target: Rect) -> bool {
        self.with_state_ref(|state| state.native.animation_targets_outer_rect(win, target))
    }

    pub(crate) fn begin_window_animation(
        &mut self,
        win: WindowId,
        from: Rect,
        to: Rect,
        duration: std::time::Duration,
    ) {
        self.with_state(|state| {
            let Some(border_width) = state
                .wm
                .core
                .state
                .model
                .client(win)
                .map(|client| client.border_width)
            else {
                return;
            };
            state.native.set_window_target_rect(
                &state.wm.core.state,
                win,
                to,
                border_width,
                crate::backend::wayland::compositor::window::animations::WindowMoveMode::AnimateFrom {
                    from,
                    duration,
                },
            );
        });
    }

    pub(crate) fn prepare_launch_environment(
        &mut self,
        command: &mut std::process::Command,
        selected_window: Option<WindowId>,
        context: crate::client::LaunchContext,
    ) {
        use smithay::wayland::seat::WaylandFocus;

        let token = self.with_state(|state| {
            let source_surface = selected_window.and_then(|win| {
                state
                    .native
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
                .native
                .xdg_activation_state
                .create_external_token(Some(token_data));
            token.as_str().to_owned()
        });
        command.env("XDG_ACTIVATION_TOKEN", token);

        if let Some(display) = self.xdisplay() {
            command.env("DISPLAY", format!(":{display}"));
        } else if let Ok(display) = std::env::var("DISPLAY") {
            command.env("DISPLAY", display);
        }
    }
}

impl WindowOps for crate::contexts::WmCtxWayland<'_> {
    fn resize_window(&mut self, window: WindowId, rect: Rect) {
        self.wayland.with_state(|state: &mut WaylandState| {
            state
                .native
                .resize_window(&state.wm.core.state, window, rect)
        });
    }

    fn set_border_width(&mut self, _window: WindowId, _width: i32) {
        // Wayland borders are compositor-rendered from core client state.
    }

    fn raise_window_visual_only(&mut self, window: WindowId) {
        self.wayland
            .with_state(|state: &mut WaylandState| state.native.raise_window_visual_only(window));
    }

    fn apply_z_order(&mut self, windows: &[WindowId]) {
        self.wayland
            .with_state(|state: &mut WaylandState| state.native.apply_z_order(windows));
    }

    fn map_window(&mut self, window: WindowId) {
        self.wayland
            .with_state(|state: &mut WaylandState| state.map_window_in_space(window));
    }

    fn unmap_window(&mut self, window: WindowId) {
        self.wayland
            .with_state(|state: &mut WaylandState| state.unmap_window_from_space(window));
    }

    fn window_exists(&self, window: WindowId) -> bool {
        self.wayland
            .with_state_ref(|state: &WaylandState| state.native.window_exists(window))
    }

    fn flush(&mut self) {
        self.wayland.with_state(WaylandState::flush);
    }

    fn window_protocol(&self, window: WindowId) -> WindowProtocol {
        self.wayland.window_protocol(window)
    }
}

impl PointerOps for WaylandBackend<'_> {
    fn pointer_location(&self) -> Option<Point> {
        WaylandBackend::pointer_location(self)
    }

    fn warp_pointer(&mut self, x: f64, y: f64) {
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
        self.wayland.with_state(|state| {
            state
                .native
                .reconcile_interactive_resize(&state.wm.core.state, desired.active_resize_window)
        });
    }
}

impl crate::backend::WindowCloseOps for crate::contexts::WmCtxWayland<'_> {
    fn close_window(&mut self, window: WindowId) {
        let _ = self.wayland.close_window(window);
    }
}

impl WaylandBackend<'_> {
    /// Project the sanitized monitor policy onto the output state.
    pub fn apply_monitor_configs(&mut self, policy: &crate::output_mirror::MonitorPolicy) {
        self.with_state(|state: &mut WaylandState| {
            let output_names: Vec<_> = state
                .native
                .output_management_state
                .outputs()
                .iter()
                .map(|output| output.name())
                .collect();
            state.native.runtime.mirror_of = policy.mirrors.clone();
            state.native.runtime.configured_output_positions.clear();
            for name in &output_names {
                let Some(config) = policy.effective(name) else {
                    continue;
                };
                if config.position.is_some() {
                    state
                        .native
                        .runtime
                        .configured_output_positions
                        .insert(name.clone());
                }
                state.native.set_output_config(name, config);
            }
            state.native.queue_output_policy_projection(&policy.configs);
        });
    }
}

impl crate::backend::OutputPolicyOps for crate::contexts::WmCtxWayland<'_> {
    fn apply_monitor_configs(&mut self, policy: &crate::output_mirror::MonitorPolicy) {
        self.wayland.apply_monitor_configs(policy);
    }
}

impl OutputOps for WaylandBackend<'_> {
    fn connected_output_names(&self) -> Vec<String> {
        self.with_state_ref(|state| {
            state
                .native
                .output_management_state
                .outputs()
                .iter()
                .map(|output| output.name())
                .collect()
        })
    }

    fn get_outputs(&self) -> Vec<crate::backend::BackendOutputInfo> {
        self.with_state_ref(|state: &WaylandState| {
            state
                .native
                .space
                .outputs()
                .map(|o| {
                    let name = o.name();
                    let geom = state.native.space.output_geometry(o).unwrap_or_default();
                    let metadata = state.native.output_vrr_metadata(&name);
                    let mut mirrors: Vec<String> = state
                        .native
                        .runtime
                        .realized_mirrors
                        .iter()
                        .filter(|(_, source)| **source == name)
                        .map(|(mirror, _)| mirror.clone())
                        .collect();
                    mirrors.sort();
                    crate::backend::BackendOutputInfo {
                        rect: crate::types::Rect {
                            x: geom.loc.x,
                            y: geom.loc.y,
                            w: geom.size.w,
                            h: geom.size.h,
                        },
                        scale: o.current_scale().fractional_scale(),
                        vrr_support: metadata
                            .map(|m| m.vrr_support)
                            .unwrap_or(crate::backend::BackendVrrSupport::Unsupported),
                        vrr_mode: metadata.map(|m| m.vrr_mode),
                        vrr_enabled: metadata.is_some_and(|m| m.vrr_enabled),
                        mirrors,
                        name,
                    }
                })
                .collect()
        })
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
        self.wayland.with_state(|state| {
            state
                .native
                .set_layout_preview_target(rect, style, target, animate, duration)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{WaylandBackend, wayland_cursor_icon};
    use crate::backend::{OutputOps, WindowProtocol};
    use crate::types::{AltCursor, ResizeDirection, WindowId};
    use smithay::input::pointer::CursorIcon;

    #[test]
    fn apply_monitor_configs_records_mirrors_without_anchoring_them() {
        use crate::config::config_toml::MonitorConfig;

        let (_event_loop, mut state) = crate::test_support::new_compositor();
        state
            .native
            .create_output("eDP-1", crate::types::Size::new(1920, 1080), None);
        state
            .native
            .create_output("DP-1", crate::types::Size::new(1920, 1080), None);
        let mut backend = WaylandBackend::new(&mut state);

        let configs = [
            (
                "eDP-1".to_string(),
                MonitorConfig {
                    position: Some("0,0".to_string()),
                    ..MonitorConfig::default()
                },
            ),
            (
                "DP-1".to_string(),
                MonitorConfig {
                    mirror: Some("eDP-1".to_string()),
                    ..MonitorConfig::default()
                },
            ),
        ]
        .into_iter()
        .collect();

        backend.apply_monitor_configs(&crate::output_mirror::MonitorPolicy::new(&configs));

        backend.with_state(|state| {
            assert_eq!(
                state.native.runtime.mirror_of.source_of("DP-1"),
                Some("eDP-1")
            );
            // A mirror owns no desktop region, so it is not a placement
            // anchor; automatic placement simply skips it.
            assert!(
                state
                    .native
                    .runtime
                    .configured_output_positions
                    .contains("eDP-1")
            );
            assert!(
                !state
                    .native
                    .runtime
                    .configured_output_positions
                    .contains("DP-1")
            );
            assert!(state.native.runtime.projected_mirrors.contains("DP-1"));
            // Roles change only once the pinning transaction applies.
            assert!(state.native.runtime.realized_mirrors.is_empty());
        });
    }

    #[test]
    fn discovery_reports_realized_mirrors_under_their_source() {
        let (_event_loop, mut state) = crate::test_support::new_compositor();
        state
            .native
            .create_output("eDP-1", crate::types::Size::new(1920, 1080), None);
        state
            .native
            .create_output("DP-1", crate::types::Size::new(1920, 1080), None);
        state
            .native
            .set_mirror_roles([("DP-1".to_string(), "eDP-1".to_string())].into());
        let backend = WaylandBackend::new(&mut state);

        let outputs = backend.get_outputs();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].name, "eDP-1");
        assert_eq!(outputs[0].mirrors, vec!["DP-1".to_string()]);
        assert_eq!(
            backend.connected_output_names().len(),
            2,
            "a mirror head is still a connected output"
        );
    }

    #[test]
    fn window_protocol_trait_dispatch_delegates_to_inherent_query() {
        let (_event_loop, mut state) = crate::test_support::new_compositor();
        state.wm = crate::wm::WaylandWm::new(crate::backend::WaylandBackendData::default());
        let ctx = state.ctx();
        let ops: &dyn crate::backend::WindowOps = &ctx;

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
