//! Shared utilities for all Wayland compositor backends.
//!
//! Runtime entry setup (WM + event loop + socket / autostart / IPC) lives in
//! [`crate::wayland::runtime::common`].
//!
//! This module contains everything that is identical between the nested
//! (winit) backend and the standalone DRM/KMS backend:
//!
//! - WM globals initialisation (`init_globals`)
//! - Session environment variables (`apply_session_env`)
//! - Wayland listening socket setup (`setup_socket`)
//! - XWayland spawn + wiring (`spawn_xwayland`)
//! - Bar buffer and shared scene-element building
//! - Frame callback dispatch (`send_frame_callbacks`)
//! - Layer shell element counting (`count_upper_layer_render_elements`)

use std::env;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::{
    Element, Id, RenderElementStates, default_primary_scanout_output_compare,
};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::utils::{
    send_frames_surface_tree, surface_primary_scanout_output,
    update_surface_primary_scanout_output, with_surfaces_surface_tree,
};
use smithay::input::keyboard::ModifiersState;
use smithay::input::pointer::{CursorIcon, CursorImageAttributes, CursorImageStatus};
use smithay::output::Output;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::with_states;
use smithay::wayland::fractional_scale::with_fractional_scale;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::xwayland::{X11Wm, XWayland, XWaylandEvent};

use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};
use crate::backend::{Backend, WaylandBackendData};
use crate::config::init_config;
use crate::contexts::CoreCtx;
use crate::core_state::CoreState;
use crate::wm::Wm;

// ─────────────────────────────────────────────────────────────────────────────
// Input helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a Smithay `ModifiersState` to an X11-style modifier bitmask.
///
/// Used by both backends to translate keyboard modifier state into the format
/// that instantWM's keybinding system expects.
pub fn modifiers_to_x11_mask(mods: &ModifiersState) -> u32 {
    let mut mask = 0u32;
    if mods.shift {
        mask |= crate::config::SHIFT;
    }
    if mods.ctrl {
        mask |= crate::config::CONTROL;
    }
    if mods.alt {
        mask |= crate::config::MOD1;
    }
    if mods.logo {
        mask |= crate::config::MODKEY;
    }
    mask
}

/// Backend-agnostic cursor state after applying WM override policy.
#[derive(Debug, PartialEq)]
pub enum CursorPresentation {
    Hidden,
    Named(CursorIcon),
    Surface {
        surface: WlSurface,
        hotspot: Point<i32, Logical>,
    },
    DndIcon {
        icon: WlSurface,
        hotspot: Point<i32, Logical>,
        cursor: Box<CursorPresentation>,
    },
}

/// Resolve effective cursor state shared by nested and DRM backends.
///
/// WM icon overrides are only visual hints for compositor-driven interactions.
/// A client-hidden cursor must remain hidden so relative pointer users, such as
/// games running through XWayland, cannot be defeated by stale hover state.
pub fn resolve_cursor_presentation(
    status: &CursorImageStatus,
    icon_override: Option<CursorIcon>,
    dnd_icon: Option<&WlSurface>,
    hidden_by_touch: bool,
) -> CursorPresentation {
    if hidden_by_touch {
        return CursorPresentation::Hidden;
    }
    let base = match status {
        CursorImageStatus::Hidden => CursorPresentation::Hidden,
        CursorImageStatus::Named(icon) => CursorPresentation::Named(icon_override.unwrap_or(*icon)),
        CursorImageStatus::Surface(surface) => {
            if let Some(icon) = icon_override {
                CursorPresentation::Named(icon)
            } else {
                // Check if the cursor surface is still alive before using it.
                // If the surface is dead, fall back to the default cursor icon.
                if !smithay::utils::IsAlive::alive(surface) {
                    return CursorPresentation::Named(CursorIcon::Default);
                }
                let hotspot = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<Mutex<CursorImageAttributes>>()
                        .and_then(|attrs| attrs.lock().ok().map(|guard| guard.hotspot))
                        .unwrap_or((0, 0).into())
                });
                CursorPresentation::Surface {
                    surface: surface.clone(),
                    hotspot,
                }
            }
        }
    };

    if let Some(icon) = dnd_icon
        && smithay::utils::IsAlive::alive(icon)
    {
        let hotspot = with_states(icon, |states| {
            states
                .data_map
                .get::<Mutex<CursorImageAttributes>>()
                .and_then(|attrs| attrs.lock().ok().map(|guard| guard.hotspot))
                .unwrap_or((0, 0).into())
        });
        return CursorPresentation::DndIcon {
            icon: icon.clone(),
            hotspot,
            cursor: Box::new(base),
        };
    }

    base
}

#[cfg(test)]
mod tests {
    use smithay::input::pointer::{CursorIcon, CursorImageStatus};

    use super::{CursorPresentation, resolve_cursor_presentation};

    #[test]
    fn hidden_cursor_status_wins_over_wm_icon_override() {
        let presentation = resolve_cursor_presentation(
            &CursorImageStatus::Hidden,
            Some(CursorIcon::Grabbing),
            None,
            false,
        );

        assert!(matches!(presentation, CursorPresentation::Hidden));
    }

    #[test]
    fn wm_icon_override_still_applies_to_named_cursor_status() {
        let presentation = resolve_cursor_presentation(
            &CursorImageStatus::Named(CursorIcon::Default),
            Some(CursorIcon::Grabbing),
            None,
            false,
        );

        assert_eq!(
            presentation,
            CursorPresentation::Named(CursorIcon::Grabbing)
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// WM globals initialisation
// ─────────────────────────────────────────────────────────────────────────────

/// Initialise all WM globals that are shared between the nested and standalone
/// Wayland backends.
///
/// Reads `config.toml`, applies tag/key configuration, sets bar metrics, and
/// calls `update_geom` so that monitor layout is valid before the first frame.
///
/// The caller is responsible for setting `wm.core.config.screen_width` /
/// `screen_height` to the actual output dimensions afterwards (e.g. from the
/// winit window size or DRM connector mode).  The values written here
/// Wayland-specific globals initialization.
///
/// Sets up config, tags, and bar painter font size. This is called before
/// the Wayland compositor is fully initialized, so monitor geometry is not
/// available yet - that will be done via update_geom later.
/// Apply font-derived bar metrics to the runtime config and bar painter.
///
/// Computes `bar_height` and `horizontal_padding` from the font config and
/// applies them to the given `CoreState`. Also updates the bar painter's font
/// size. Shared by both startup (`init_globals`) and reload.
pub fn apply_bar_metrics(g: &mut CoreState, data: &mut WaylandBackendData) {
    let font_size = g.config.fonts.size();
    let font_families = g.config.fonts.families();
    let metrics = g.config.fonts.bar_metrics(g.config.bar.height);

    data.bar_painter.set_font_size(font_size);
    data.bar_painter.set_font_families(&font_families);

    g.config.derived.bar_height = metrics.height;
    g.config.derived.bar_horizontal_padding = metrics.horizontal_padding;
}

pub fn init_globals(g: &mut CoreState, wayland: &mut WaylandBackendData) {
    let cfg = init_config(crate::backend::BackendKind::Wayland);
    g.config.derived.display.width = 1280;
    g.config.derived.display.height = 800;
    crate::core_state::apply_config(g, &cfg);
    g.config.bar.show = true;

    apply_bar_metrics(g, wayland);

    // Monitor geometry will be set up after the compositor is ready via update_geom
}

// ─────────────────────────────────────────────────────────────────────────────
// Session environment
// ─────────────────────────────────────────────────────────────────────────────

/// Set the standard environment variables that tell toolkit clients how to
/// connect to this compositor.
///
/// Called after the Wayland socket name is known.  Both the nested backend
/// (which merely exports `WAYLAND_DISPLAY` into the nested environment) and
/// the standalone DRM backend (which is the actual session compositor) use the
/// same set of variables.
pub fn apply_session_env(socket_name: &str) {
    unsafe {
        env::set_var("WAYLAND_DISPLAY", socket_name);
        env::set_var("XDG_SESSION_TYPE", "wayland");
        env::set_var("XDG_CURRENT_DESKTOP", "instantwm");
        env::set_var("XDG_SESSION_DESKTOP", "instantwm");
        env::set_var("DESKTOP_SESSION", "instantwm");
        env::remove_var("DISPLAY");
        env::set_var("GDK_BACKEND", "wayland");
        env::set_var("QT_QPA_PLATFORM", "wayland");
        env::set_var("SDL_VIDEODRIVER", "wayland");
        env::set_var("CLUTTER_BACKEND", "wayland");
    }
}

pub fn ensure_dbus_session() {
    if env::var("DBUS_SESSION_BUS_ADDRESS").is_ok() {
        return;
    }

    let Ok(output) = Command::new("dbus-daemon")
        .arg("--session")
        .arg("--fork")
        .arg("--print-address=1")
        .arg("--nopidfile")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    else {
        log::warn!("dbus-daemon not found, D-Bus session bus unavailable");
        return;
    };

    let addr = String::from_utf8_lossy(&output.stdout);
    let addr = addr.trim();
    if !addr.is_empty() {
        unsafe { env::set_var("DBUS_SESSION_BUS_ADDRESS", addr) };
        log::info!("Started D-Bus session bus: {addr}");
    }
}

/// Import the Wayland session environment into the D-Bus activation environment.
///
/// Portals and other D-Bus-activated services need these variables to discover
/// the compositor socket and desktop identity. This mirrors the environment
/// import step commonly done by compositor session wrappers.
pub fn import_env_into_dbus_activation() {
    let mut attempted = false;

    if let Ok(status) = Command::new("dbus-update-activation-environment")
        .arg("--systemd")
        .arg("WAYLAND_DISPLAY")
        .arg("XDG_CURRENT_DESKTOP")
        .arg("XDG_SESSION_DESKTOP")
        .arg("DESKTOP_SESSION")
        .status()
    {
        attempted = true;
        if !status.success() {
            log::debug!(
                "dbus-update-activation-environment exited with status {}",
                status
            );
        }
    }

    // Fall back to the non-systemd import path when systemd integration is
    // unavailable.
    if !attempted {
        match Command::new("dbus-update-activation-environment")
            .arg("WAYLAND_DISPLAY")
            .arg("XDG_CURRENT_DESKTOP")
            .arg("XDG_SESSION_DESKTOP")
            .arg("DESKTOP_SESSION")
            .status()
        {
            Ok(status) if !status.success() => log::debug!(
                "dbus-update-activation-environment exited with status {}",
                status
            ),
            Ok(_) => {}
            Err(err) => log::debug!("dbus-update-activation-environment unavailable: {}", err),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Wayland socket
// ─────────────────────────────────────────────────────────────────────────────

/// Create an auto-named Wayland listening socket, register it with the calloop
/// event loop so that new client connections are accepted automatically, and
/// apply the session environment.
///
/// Returns the socket name (e.g. `"wayland-1"`) so callers can log it or pass
/// it to child processes.
pub fn setup_socket(
    loop_handle: &LoopHandle<'static, WaylandState>,
    state: &WaylandState,
) -> String {
    let listening_socket = ListeningSocketSource::new_auto().expect("wayland socket");
    let socket_name = listening_socket
        .socket_name()
        .to_string_lossy()
        .into_owned();

    apply_session_env(&socket_name);
    import_env_into_dbus_activation();

    loop_handle
        .insert_source(listening_socket, |client, _, data| {
            let _ = data
                .display_handle
                .insert_client(client, Arc::new(WaylandClientState::default()));
        })
        .expect("listening socket source");

    let _ = state; // reserved for future use (e.g. security policy)
    socket_name
}

// ─────────────────────────────────────────────────────────────────────────────
// XWayland
// ─────────────────────────────────────────────────────────────────────────────

/// Spawn XWayland and wire its calloop source into the event loop.
///
/// On success, `DISPLAY` is immediately set to the pre-assigned display number
/// so that any autostart processes that check the environment see it right away.
/// The definitive `DISPLAY` value is set again inside the `XWaylandEvent::Ready`
/// callback once XWayland confirms its display number.
///
/// Errors are logged and silently swallowed: a missing XWayland is non-fatal
/// (pure Wayland clients still work).
pub fn spawn_xwayland(state: &WaylandState, loop_handle: &LoopHandle<'static, WaylandState>) {
    match XWayland::spawn(
        &state.display_handle,
        None,
        std::iter::empty::<(String, String)>(),
        true,
        Stdio::null(),
        Stdio::null(),
        |_| (),
    ) {
        Ok((xwayland, client)) => {
            unsafe { env::set_var("DISPLAY", format!(":{}", xwayland.display_number())) };
            let handle_for_wm = loop_handle.clone();
            if let Err(err) = loop_handle.insert_source(xwayland, move |event, _, data| match event
            {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => {
                    data.xdisplay = Some(display_number);
                    unsafe { env::set_var("DISPLAY", format!(":{display_number}")) };
                    match X11Wm::start_wm(
                        handle_for_wm.clone(),
                        &data.display_handle,
                        x11_socket,
                        client.clone(),
                    ) {
                        Ok(wm) => data.xwm = Some(wm),
                        Err(e) => log::error!("failed to start X11 WM for XWayland: {e}"),
                    }
                }
                XWaylandEvent::Error => {
                    log::error!("XWayland failed to start");
                }
            }) {
                log::error!("failed to insert XWayland source: {err}");
            }
        }
        Err(err) => {
            log::warn!("failed to spawn XWayland: {err}");
        }
    }
}

/// Spawn a lightweight test window a short time after startup.
///
/// This gives the compositor something visible to display immediately after
/// launch during development / smoke-testing. Set
/// `INSTANTWM_WL_AUTOSPAWN=0` to suppress it.
pub fn spawn_smoke_window() {
    if env::var("INSTANTWM_WL_AUTOSTART").ok().as_deref() == Some("0") {
        return;
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(800));
        let _ = Command::new("sh")
            .arg("-lc")
            .arg("for app in gtk3-demo thunar xmessage; do command -v \"$app\" >/dev/null 2>&1 && exec \"$app\"; done; exit 0")
            .spawn();
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Bar render elements
// ─────────────────────────────────────────────────────────────────────────────

/// Build the `MemoryRenderBufferRenderElement` list for the status bar.
///
/// Returns an empty `Vec` when `wm.core.config.showbar` is `false`.
///
/// The caller is responsible for adding the returned elements to its own
/// custom-element list under the appropriate backend-specific wrapper variant
/// (e.g. `DrmExtras::Memory` or `WaylandExtras::Memory`).
pub fn build_bar_buffers(
    wm: &mut Wm,
    state: &mut WaylandState,
) -> Vec<(MemoryRenderBuffer, crate::types::Point)> {
    if !wm.core.config.bar.show {
        return Vec::new();
    }

    let tray_menu = wm.tray_menu.presentation();
    let mut core = CoreCtx::new(
        &mut wm.core,
        &mut wm.work,
        &mut wm.running,
        &mut wm.bar,
        &mut wm.focus,
    );

    {
        let Backend::Wayland(data) = &mut wm.backend else {
            return Vec::new();
        };

        data.bar_painter
            .set_render_ping(state.runtime.render_ping.clone());
        crate::bar::wayland::render_bar_buffers(
            &mut core,
            &mut data.bar_painter,
            smithay::utils::Scale::from(1.0),
            &data.status_notifier_tray,
            tray_menu.as_ref(),
        )
    }
}

/// Poll Wayland systray events once and mark the bar dirty when icons changed.
pub fn poll_systray(wm: &mut Wm) {
    let core = CoreCtx::new(
        &mut wm.core,
        &mut wm.work,
        &mut wm.running,
        &mut wm.bar,
        &mut wm.focus,
    );
    let Backend::Wayland(data) = &mut wm.backend else {
        return;
    };

    if let Some(runtime) = data.status_notifier_runtime.as_mut() {
        let dirty = runtime.poll_events(&mut data.status_notifier_tray, &mut wm.tray_menu);
        if dirty {
            core.bar.mark_dirty();
        }
    }
}

/// Shared render elements that are not output-local and can be reused across
/// multiple output renders in the same frame.
#[derive(Clone)]
pub struct FixedSceneElements {
    pub bar_buffers: Vec<(MemoryRenderBuffer, crate::types::Point)>,
    pub borders: Vec<SolidColorRenderElement>,
    pub layout_preview_color: crate::bar::color::Rgba,
}

/// Build the shared scene pieces that do not depend on the target output.
pub fn build_fixed_scene_elements(wm: &mut Wm, state: &mut WaylandState) -> Rc<FixedSceneElements> {
    let bar_seq = wm.bar.update_seq();
    let borders_hash = crate::wayland::render::borders::get_borders_hash(&wm.core.model, state);

    if !wm.bar.needs_redraw()
        && let Some((cached_bar, cached_borders, ref elements)) = state.runtime.fixed_scene_cache
        && cached_bar == bar_seq
        && cached_borders == borders_hash
    {
        return elements.clone();
    }

    let elements = Rc::new(FixedSceneElements {
        bar_buffers: build_bar_buffers(wm, state),
        borders: crate::wayland::render::borders::render_border_elements(
            &wm.core.model,
            &wm.core.config.colors.border,
            state,
        ),
        layout_preview_color: wm.core.config.colors.border.snap,
    });

    if !wm.bar.needs_redraw() {
        state.runtime.fixed_scene_cache = Some((bar_seq, borders_hash, elements.clone()));
    }
    elements
}

/// Backend-agnostic render element buckets used by both Wayland startup paths.
pub struct CommonSceneElements {
    pub overlays: Vec<WaylandSurfaceRenderElement<GlesRenderer>>,
    pub bar: Vec<MemoryRenderBufferRenderElement<GlesRenderer>>,
    pub borders: Vec<SolidColorRenderElement>,
}

/// Build the shared set of scene extras used by both startup renderers.
pub fn build_common_scene_elements(
    wm: &mut Wm,
    state: &mut WaylandState,
    renderer: &mut GlesRenderer,
    output: &Output,
) -> CommonSceneElements {
    let fixed = build_fixed_scene_elements(wm, state);
    build_common_scene_elements_from_fixed(state, renderer, output, &fixed)
}

/// Build the full scene for one output from reusable shared pieces.
pub fn build_common_scene_elements_from_fixed(
    state: &WaylandState,
    renderer: &mut GlesRenderer,
    output: &Output,
    fixed: &FixedSceneElements,
) -> CommonSceneElements {
    use smithay::backend::renderer::element::AsRenderElements;

    let mut overlays = Vec::new();
    for (window, phys_loc) in state.overlay_windows_for_render(output) {
        let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            AsRenderElements::render_elements(
                &window,
                renderer,
                phys_loc,
                smithay::utils::Scale::from(1.0),
                1.0,
            );
        overlays.extend(elems);
    }

    let mut bar = Vec::new();
    for (buffer, position) in &fixed.bar_buffers {
        match MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            (position.x as f64, position.y as f64),
            buffer,
            None,
            None,
            None,
            Kind::Unspecified,
        ) {
            Ok(elem) => bar.push(elem),
            Err(e) => log::warn!("bar buffer upload failed: {:?}", e),
        }
    }

    let mut borders = fixed.borders.clone();
    crate::wayland::render::borders::append_layout_preview(
        &mut borders,
        state.layout_preview_rect(),
        fixed.layout_preview_color,
    );

    CommonSceneElements {
        overlays,
        bar,
        borders,
    }
}

/// Remove the Smithay-space copies of windows already emitted in the explicit
/// above-bar overlay bucket. Surface render-element IDs are stable across both
/// paths, so this avoids drawing the same surface tree twice.
pub fn remove_duplicate_overlay_elements<E: Element>(
    scene: &CommonSceneElements,
    space_elements: &mut Vec<E>,
) {
    if scene.overlays.is_empty() {
        return;
    }
    let overlay_ids: Vec<Id> = scene
        .overlays
        .iter()
        .map(|element| element.id().clone())
        .collect();
    space_elements.retain(|element| !overlay_ids.iter().any(|id| id == element.id()));
}

// ─────────────────────────────────────────────────────────────────────────────
// Frame callbacks
// ─────────────────────────────────────────────────────────────────────────────

/// Send `wl_surface.frame` callbacks for windows visible on `output`.
///
/// Must be called once per rendered frame, after the buffer has been submitted
/// for scanout, so that clients know when to draw the next frame.
///
/// `Window::send_frame` owns surface-tree and popup traversal. Window/output
/// selection is done from current geometry rather than `Space`'s cached output
/// membership: commits can arrive before the next `Space::refresh`, especially
/// for short-lived Xwayland override-redirect windows.
pub fn send_frame_callbacks(state: &WaylandState, output: &Output, elapsed: Duration) {
    let throttle = output.current_mode().and_then(|mode| {
        let refresh = u64::try_from(mode.refresh).ok()?;
        (refresh > 0).then(|| Duration::from_nanos(1_000_000_000_000u64 / refresh))
    });

    if state.is_locked() {
        let output_name = output.name();
        if let Some(lock_surface) = state.lock_surfaces.get(&output_name) {
            send_frames_surface_tree(
                lock_surface.wl_surface(),
                output,
                elapsed,
                throttle,
                surface_primary_scanout_output,
            );
        }
        return;
    }

    for window in state
        .space
        .elements()
        .filter(|window| window_overlaps_output(state, window, output))
    {
        window.send_frame(output, elapsed, throttle, surface_primary_scanout_output);
    }

    // Layer surfaces for this output only.
    let map = smithay::desktop::layer_map_for_output(output);
    for layer_surface in map.layers() {
        layer_surface.send_frame(output, elapsed, throttle, surface_primary_scanout_output);
    }
}

/// Update Smithay's primary-scanout bookkeeping for all surfaces visible on `output`.
///
/// `send_frames_surface_tree` and presentation feedback use this state to decide
/// which output should drive a surface's callbacks. If we never update it,
/// frame callbacks are throttled as if every surface were off-screen, which can
/// stall clients that rely on `wl_surface.frame`.
pub fn update_primary_scanout_output(
    state: &WaylandState,
    output: &Output,
    render_states: &RenderElementStates,
) {
    if state.is_locked() {
        let output_name = output.name();
        if let Some(lock_surface) = state.lock_surfaces.get(&output_name) {
            with_surfaces_surface_tree(lock_surface.wl_surface(), |surface, data| {
                let _ = update_surface_primary_scanout_output(
                    surface,
                    output,
                    data,
                    None,
                    render_states,
                    default_primary_scanout_output_compare,
                );
                update_preferred_fractional_scale(surface, data);
            });
        }
        return;
    }

    for window in state
        .space
        .elements()
        .filter(|window| window_overlaps_output(state, window, output))
    {
        window.with_surfaces(|surface, data| {
            let _ = update_surface_primary_scanout_output(
                surface,
                output,
                data,
                None,
                render_states,
                default_primary_scanout_output_compare,
            );
            update_preferred_fractional_scale(surface, data);
        });
    }

    let map = smithay::desktop::layer_map_for_output(output);
    for layer_surface in map.layers() {
        layer_surface.with_surfaces(|surface, data| {
            let _ = update_surface_primary_scanout_output(
                surface,
                output,
                data,
                None,
                render_states,
                default_primary_scanout_output_compare,
            );
            update_preferred_fractional_scale(surface, data);
        });
    }
}

fn update_preferred_fractional_scale(
    surface: &WlSurface,
    states: &smithay::wayland::compositor::SurfaceData,
) {
    let Some(output) = surface_primary_scanout_output(surface, states) else {
        return;
    };
    with_fractional_scale(states, |fractional_scale| {
        fractional_scale.set_preferred_scale(output.current_scale().fractional_scale());
    });
}

/// Test current compositor geometry instead of Smithay's lazily refreshed
/// element/output membership cache.
pub(crate) fn window_overlaps_output(
    state: &WaylandState,
    window: &smithay::desktop::Window,
    output: &Output,
) -> bool {
    let Some(output_rect) = state.space.output_geometry(output) else {
        return false;
    };
    let Some(location) = state.space.element_location(window) else {
        return false;
    };
    let mut window_rect = window.bbox_with_popups();
    window_rect.loc += location - window.geometry().loc;
    output_rect.overlaps(window_rect)
}

// ─────────────────────────────────────────────────────────────────────────────
// Misc
// ─────────────────────────────────────────────────────────────────────────────

/// Clamp output dimensions to a safe minimum so that Smithay never sees a
/// zero-sized surface.
pub fn sanitize_size(size: crate::types::Size) -> crate::types::Size {
    const WAYLAND_MIN_DIM: i32 = 64;
    crate::types::Size::new(size.w.max(WAYLAND_MIN_DIM), size.h.max(WAYLAND_MIN_DIM))
}

pub fn output_has_real_fullscreen(wm: &Wm, output: &Output) -> bool {
    let output_name = output.name();
    let Some(monitor) = wm
        .core
        .model
        .monitors
        .iter_all()
        .find(|m| m.name == output_name)
    else {
        return false;
    };
    let selected_tags = monitor.selected_tags();
    monitor
        .iter_clients(&wm.core.model.clients)
        .any(|(_, client)| client.mode().is_true_fullscreen() && client.is_visible(selected_tags))
}

// ─────────────────────────────────────────────────────────────────────────────
// Layer shell rendering helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Count the number of render elements in upper layer shells (Overlay/Top).
///
/// This is used by both backends to determine how many space render elements
/// to place before the bar and borders.
pub fn count_upper_layer_render_elements(renderer: &mut GlesRenderer, output: &Output) -> usize {
    let layer_map = smithay::desktop::layer_map_for_output(output);
    let output_scale = output.current_scale().fractional_scale();
    let mut num_upper = 0;

    for surface in layer_map.layers().rev() {
        if matches!(
            surface.layer(),
            smithay::wayland::shell::wlr_layer::Layer::Background
                | smithay::wayland::shell::wlr_layer::Layer::Bottom
        ) {
            continue;
        }
        if let Some(geo) = layer_map.layer_geometry(surface) {
            let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                smithay::backend::renderer::element::AsRenderElements::render_elements(
                    surface,
                    renderer,
                    geo.loc.to_physical_precise_round(output_scale),
                    smithay::utils::Scale::from(output_scale),
                    1.0,
                );
            num_upper += elems.len();
        }
    }

    num_upper
}

/// Helper struct to track element counts for pre-allocating the render vector.
#[derive(Default)]
pub struct RenderElementCounts {
    pub overlays: usize,
    pub upper_layers: usize,
    pub bar: usize,
    pub borders: usize,
    pub space: usize,
}

impl RenderElementCounts {
    /// Calculate total capacity needed.
    pub fn total(&self) -> usize {
        self.overlays + self.upper_layers + self.bar + self.borders + self.space
    }
}

/// Get the render element counts for a frame.
///
/// This helps pre-allocate the render element vector with the right capacity.
pub fn get_render_element_counts(
    scene: &CommonSceneElements,
    space_render_elements_len: usize,
    num_upper: usize,
) -> RenderElementCounts {
    RenderElementCounts {
        overlays: scene.overlays.len(),
        upper_layers: num_upper,
        bar: scene.bar.len(),
        borders: scene.borders.len(),
        space: space_render_elements_len.saturating_sub(num_upper),
    }
}
