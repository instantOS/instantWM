//! Shared startup utilities for both Wayland compositor backends.
//!
//! This module contains everything that is identical between the nested
//! (winit) backend and the standalone DRM/KMS backend:
//!
//! - WM globals initialisation (`init_wayland_globals`)
//! - Session environment variables (`apply_wayland_session_env`)
//! - Wayland listening socket setup (`setup_wayland_socket`)
//! - XWayland spawn + wiring (`spawn_xwayland`)
//! - Bar render-element building (`build_bar_elements`)
//! - Frame callback dispatch (`send_frame_callbacks`)

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::utils::{send_frames_surface_tree, surface_primary_scanout_output};
use smithay::desktop::PopupManager;
use smithay::input::keyboard::ModifiersState;
use smithay::input::pointer::{CursorIcon, CursorImageAttributes, CursorImageStatus};
use smithay::output::Output;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::with_states;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::xwayland::{X11Wm, XWayland, XWaylandEvent};

use crate::backend::wayland::compositor::WindowIdMarker;
use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};
use crate::config::init_config;
use crate::contexts::CoreCtx;
use crate::monitor::update_geom;
use crate::types::WindowId;
use crate::types::{CLOSE_BUTTON_DETAIL, CLOSE_BUTTON_WIDTH};
use crate::wm::Wm;

// ─────────────────────────────────────────────────────────────────────────────
// Font / text helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Extract font size from a list of font descriptor strings.
///
/// Looks for a `size=N` fragment in each string, returning the first valid
/// positive float found.  Falls back to `14.0` when nothing matches.
pub fn wayland_font_size_from_config(fonts: &[String]) -> f32 {
    fonts
        .iter()
        .find_map(|font| {
            let idx = font.find("size=")?;
            let tail = &font[idx + 5..];
            let num: String = tail
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            num.parse::<f32>().ok().filter(|s| *s > 0.0)
        })
        .unwrap_or(14.0)
}

/// Calculate a comfortable line/cell height (in pixels) from a font size.
pub fn wayland_font_height_from_size(font_size: f32) -> i32 {
    ((font_size * 1.3).ceil() as i32).max(font_size.ceil() as i32 + 2)
}

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
pub enum CursorPresentation {
    Hidden,
    Named(CursorIcon),
    Surface {
        surface: WlSurface,
        hotspot: Point<i32, Logical>,
    },
}

/// Resolve effective cursor state shared by nested and DRM backends.
///
/// WM icon overrides have priority over client-provided cursor images.
pub fn resolve_cursor_presentation(
    status: &CursorImageStatus,
    icon_override: Option<CursorIcon>,
) -> CursorPresentation {
    if let Some(icon) = icon_override {
        return CursorPresentation::Named(icon);
    }

    match status {
        CursorImageStatus::Hidden => CursorPresentation::Hidden,
        CursorImageStatus::Named(icon) => CursorPresentation::Named(*icon),
        CursorImageStatus::Surface(surface) => {
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
/// The caller is responsible for setting `wm.g.cfg.screen_width` /
/// `screen_height` to the actual output dimensions afterwards (e.g. from the
/// winit window size or DRM connector mode).  The values written here
/// (`1280 × 800`) are a safe placeholder that will be overwritten.
pub fn init_wayland_globals(wm: &mut Wm) {
    let cfg = init_config();
    wm.g.cfg.screen_width = 1280;
    wm.g.cfg.screen_height = 800;
    crate::globals::apply_config(&mut wm.g, &cfg);
    crate::globals::apply_tags_config(&mut wm.g, &cfg);
    wm.g.cfg.show_bar = true;
    let font_size = wayland_font_size_from_config(&cfg.fonts);
    let font_height = wayland_font_height_from_size(font_size);
    wm.bar_painter.set_font_size(font_size);
    let min_bar_height = CLOSE_BUTTON_WIDTH + CLOSE_BUTTON_DETAIL + 2;
    wm.g.cfg.bar_height = (if cfg.bar_height > 0 {
        font_height + cfg.bar_height
    } else {
        font_height + 12
    })
    .max(min_bar_height);
    wm.g.cfg.horizontal_padding = font_height;
    wm.x11_runtime.numlockmask = 0;
    update_geom(&mut wm.ctx());
    if !wm.g.cfg.monitors.is_empty() {
        let mut ctx = wm.ctx();
        crate::monitor::apply_monitor_config(&mut ctx);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Session environment
// ─────────────────────────────────────────────────────────────────────────────

/// Set the standard environment variables that tell toolkit clients how to
/// connect to this compositor.
///
/// Called after the Wayland socket name is known.  Both the nested backend
/// (which merely exports `WAYLAND_DISPLAY` into the nested environment) and the
/// standalone DRM backend (which is the actual session compositor) use the same
/// set of variables.
pub fn apply_wayland_session_env(socket_name: &str) {
    std::env::set_var("WAYLAND_DISPLAY", socket_name);
    std::env::set_var("XDG_SESSION_TYPE", "wayland");
    std::env::remove_var("DISPLAY");
    std::env::set_var("GDK_BACKEND", "wayland");
    std::env::set_var("QT_QPA_PLATFORM", "wayland");
    std::env::set_var("SDL_VIDEODRIVER", "wayland");
    std::env::set_var("CLUTTER_BACKEND", "wayland");
}

pub fn ensure_dbus_session() {
    if std::env::var("DBUS_SESSION_BUS_ADDRESS").is_ok() {
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
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", addr);
        log::info!("Started D-Bus session bus: {addr}");
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
pub fn setup_wayland_socket(
    loop_handle: &LoopHandle<'static, WaylandState>,
    state: &WaylandState,
) -> String {
    let listening_socket = ListeningSocketSource::new_auto().expect("wayland socket");
    let socket_name = listening_socket
        .socket_name()
        .to_string_lossy()
        .into_owned();

    apply_wayland_session_env(&socket_name);

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
            std::env::set_var("DISPLAY", format!(":{}", xwayland.display_number()));
            let handle_for_wm = loop_handle.clone();
            if let Err(err) = loop_handle.insert_source(xwayland, move |event, _, data| match event
            {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => {
                    data.xdisplay = Some(display_number);
                    std::env::set_var("DISPLAY", format!(":{display_number}"));
                    match X11Wm::start_wm(handle_for_wm.clone(), x11_socket, client.clone()) {
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
pub fn spawn_wayland_smoke_window() {
    if std::env::var("INSTANTWM_WL_AUTOSPAWN").ok().as_deref() == Some("0") {
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
/// Returns an empty `Vec` when `wm.g.cfg.showbar` is `false`.
///
/// The caller is responsible for adding the returned elements to its own
/// custom-element list under the appropriate backend-specific wrapper variant
/// (e.g. `DrmExtras::Memory` or `WaylandExtras::Memory`).
pub fn build_bar_elements(
    wm: &mut Wm,
    renderer: &mut GlesRenderer,
) -> Vec<MemoryRenderBufferRenderElement<GlesRenderer>> {
    if !wm.g.cfg.show_bar {
        return Vec::new();
    }
    if let Some(runtime) = wm.wayland_systray_runtime.as_ref() {
        let mut core = CoreCtx::new(&mut wm.g, &mut wm.running, &mut wm.bar, &mut wm.focus);
        if runtime.poll_events(
            &mut core,
            &mut wm.wayland_systray,
            &mut wm.wayland_systray_menu,
        ) {
            core.bar.mark_dirty();
        }
    }

    let mut core = CoreCtx::new(&mut wm.g, &mut wm.running, &mut wm.bar, &mut wm.focus);
    let bar_buffers = crate::bar::wayland::render_bar_buffers(
        &mut core,
        &mut wm.bar_painter,
        smithay::utils::Scale::from(1.0),
        &wm.wayland_systray,
        wm.wayland_systray_menu.as_ref(),
    );
    let mut elements = Vec::new();
    for (buffer, x, y) in bar_buffers {
        match MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            (x as f64, y as f64),
            &buffer,
            None,
            None,
            None,
            Kind::Unspecified,
        ) {
            Ok(elem) => elements.push(elem),
            Err(e) => log::warn!("bar buffer upload failed: {:?}", e),
        }
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
    state: &WaylandState,
    renderer: &mut GlesRenderer,
    output_x_offset: i32,
) -> CommonSceneElements {
    use smithay::backend::renderer::element::AsRenderElements;

    let mut overlays = Vec::new();
    for (window, phys_loc) in state.overlay_windows_for_render(output_x_offset) {
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

    let bar = build_bar_elements(wm, renderer);
    let borders = wayland_border_elements_shared(&wm.g, state);

    CommonSceneElements {
        overlays,
        bar,
        borders,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Frame callbacks
// ─────────────────────────────────────────────────────────────────────────────

/// Send `wl_surface.frame` callbacks for windows visible on `output`.
///
/// Must be called once per rendered frame, after the buffer has been submitted
/// for scanout, so that clients know when to draw the next frame.
///
/// Only windows whose geometry intersects the output receive callbacks,
/// preventing off-screen windows from committing empty-damage frames in a
/// busy loop (an approach borrowed from niri's per-output frame throttling).
pub fn send_frame_callbacks(state: &WaylandState, output: &Output, elapsed: Duration) {
    let output_geo = state.space.output_geometry(output);

    for window in state.space.elements() {
        // Only notify windows that are actually visible on this output.
        if let Some(out_geo) = output_geo {
            if let Some(win_loc) = state.space.element_location(window) {
                let win_size = window.geometry().size;
                let win_rect = smithay::utils::Rectangle::new(win_loc, win_size);
                if !out_geo.overlaps(win_rect) {
                    continue;
                }
            }
        }

        if let Some(wl_surface) = window.wl_surface() {
            send_frames_surface_tree(
                &wl_surface,
                output,
                elapsed,
                Some(Duration::from_millis(16)),
                surface_primary_scanout_output,
            );
            if let Some(toplevel) = window.toplevel() {
                for (popup, _) in PopupManager::popups_for_surface(toplevel.wl_surface()) {
                    send_frames_surface_tree(
                        popup.wl_surface(),
                        output,
                        elapsed,
                        Some(Duration::from_millis(16)),
                        surface_primary_scanout_output,
                    );
                }
            }
        }
    }

    // Layer surfaces for this output only.
    let map = smithay::desktop::layer_map_for_output(output);
    for layer_surface in map.layers() {
        layer_surface.send_frame(
            output,
            elapsed,
            Some(Duration::from_millis(16)),
            surface_primary_scanout_output,
        );
    }
}

/// Information about a window needed for border rendering.
#[derive(Debug, Clone, Copy)]
struct WindowBorderInfo {
    id: WindowId,
    geo: crate::types::Rect,
    border_width: i32,
    content_size: (i32, i32),
    is_visible: bool,
    is_hidden: bool,
    is_floating: bool,
    is_tiling_layout: bool,
}

impl WindowBorderInfo {
    /// Total outer size including borders.
    fn outer_size(&self) -> (i32, i32) {
        let bw = self.border_width;
        let (cw, ch) = self.content_size;
        (cw + 2 * bw, ch + 2 * bw)
    }

    /// Bounding rectangle including borders.
    fn bounding_rect(&self) -> IntRect {
        let (ow, oh) = self.outer_size();
        IntRect {
            x: self.geo.x,
            y: self.geo.y,
            w: ow,
            h: oh,
        }
    }

    /// Checks if this window should render borders.
    fn has_borders(&self) -> bool {
        self.is_visible && !self.is_hidden && self.border_width > 0
    }

    /// Returns the border color based on focus state.
    fn border_color(&self, is_focused: bool, colors: &crate::config::appearance::BorderColors) -> [f32; 4] {
        if is_focused {
            if self.is_floating || !self.is_tiling_layout {
                colors.float_focus
            } else {
                colors.tile_focus
            }
        } else {
            colors.normal
        }
    }
}

/// Collects window information from the compositor state.
fn collect_window_info(
    g: &crate::globals::Globals,
    state: &WaylandState,
) -> Vec<WindowBorderInfo> {
    let mut windows = Vec::new();

    for window in state.space.elements() {
        let Some(marker) = window.user_data().get::<WindowIdMarker>() else {
            continue;
        };
        let Some(c) = g.clients.get(&marker.id) else {
            continue;
        };

        let size = window.geometry().size;
        let content_size = (size.w.max(1), size.h.max(1));

        let is_visible = g
            .monitor(c.monitor_id)
            .map(|m| c.is_visible_on_tags(m.selected_tags()))
            .unwrap_or(false);

        let is_tiling_layout = g
            .monitor(c.monitor_id)
            .map(|m| m.is_tiling_layout())
            .unwrap_or(true);

        windows.push(WindowBorderInfo {
            id: marker.id,
            geo: c.geo,
            border_width: c.border_width.max(0),
            content_size,
            is_visible,
            is_hidden: c.is_hidden,
            is_floating: c.is_floating,
            is_tiling_layout,
        });
    }

    windows
}

/// Generates the four border rectangles for a window.
fn generate_border_rectangles(x: i32, y: i32, outer_w: i32, outer_h: i32, bw: i32) -> Vec<IntRect> {
    if bw <= 0 || outer_w <= 2 * bw || outer_h <= 2 * bw {
        return Vec::new();
    }

    let inner_h = (outer_h - 2 * bw).max(0);

    vec![
        // Top border
        IntRect { x, y, w: outer_w, h: bw },
        // Bottom border
        IntRect {
            x,
            y: y + outer_h - bw,
            w: outer_w,
            h: bw,
        },
        // Left border (between top and bottom)
        IntRect {
            x,
            y: y + bw,
            w: bw,
            h: inner_h,
        },
        // Right border (between top and bottom)
        IntRect {
            x: x + outer_w - bw,
            y: y + bw,
            w: bw,
            h: inner_h,
        },
    ]
}

/// Subtracts occluders from border parts, returning the remaining visible parts.
fn apply_occluders(border_parts: Vec<IntRect>, occluders: &[IntRect]) -> Vec<IntRect> {
    let mut remaining = border_parts;

    for occluder in occluders {
        if remaining.is_empty() {
            break;
        }
        remaining = remaining
            .into_iter()
            .flat_map(|part| subtract_rect(part, *occluder))
            .collect();
    }

    remaining
}

/// Builds occluder rectangles from windows (windows block borders behind them).
fn build_occluders(windows: &[WindowBorderInfo]) -> Vec<IntRect> {
    windows
        .iter()
        .filter(|w| w.is_visible)
        .map(|w| w.bounding_rect())
        .collect()
}

/// Renders border elements for all visible windows.
pub(crate) fn wayland_border_elements_shared(
    g: &crate::globals::Globals,
    state: &WaylandState,
) -> Vec<SolidColorRenderElement> {
    let windows = collect_window_info(g, state);
    let selected_win = g.selected_win();
    let colors = &g.cfg.bordercolors;
    let mut elements = Vec::new();

    // Build occluders list (each window can occlude borders behind it)
    let occluders: Vec<IntRect> = build_occluders(&windows);

    for (idx, window) in windows.iter().enumerate() {
        if !window.has_borders() {
            continue;
        }

        let (outer_w, outer_h) = window.outer_size();
        let bw = window.border_width;

        // Generate the four border sides
        let border_parts = generate_border_rectangles(window.geo.x, window.geo.y, outer_w, outer_h, bw);
        if border_parts.is_empty() {
            continue;
        }

        // Subtract occluders from higher windows (windows in front)
        let higher_occluders = &occluders[idx + 1..];
        let visible_parts = apply_occluders(border_parts, higher_occluders);

        // Get color based on focus state
        let is_focused = Some(window.id) == selected_win;
        let color = window.border_color(is_focused, colors);

        // Create render elements for visible border parts
        for part in visible_parts {
            push_solid(&mut elements, part.x, part.y, part.w, part.h, color);
        }
    }

    elements
}

// ─────────────────────────────────────────────────────────────────────────────
// Misc
// ─────────────────────────────────────────────────────────────────────────────

/// Clamp output dimensions to a safe minimum so that Smithay never sees a
/// zero-sized surface.
pub fn sanitize_wayland_size(w: i32, h: i32) -> (i32, i32) {
    const WAYLAND_MIN_DIM: i32 = 64;
    (w.max(WAYLAND_MIN_DIM), h.max(WAYLAND_MIN_DIM))
}

#[derive(Clone, Copy)]
struct IntRect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

fn intersect_rect(a: IntRect, b: IntRect) -> Option<IntRect> {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = (a.x + a.w).min(b.x + b.w);
    let y2 = (a.y + a.h).min(b.y + b.h);
    if x2 <= x1 || y2 <= y1 {
        return None;
    }
    Some(IntRect {
        x: x1,
        y: y1,
        w: x2 - x1,
        h: y2 - y1,
    })
}

fn subtract_rect(base: IntRect, cut: IntRect) -> Vec<IntRect> {
    if base.w <= 0 || base.h <= 0 {
        return Vec::new();
    }
    let Some(i) = intersect_rect(base, cut) else {
        return vec![base];
    };

    let mut out = Vec::new();
    if i.y > base.y {
        out.push(IntRect {
            x: base.x,
            y: base.y,
            w: base.w,
            h: i.y - base.y,
        });
    }
    let base_bottom = base.y + base.h;
    let inter_bottom = i.y + i.h;
    if inter_bottom < base_bottom {
        out.push(IntRect {
            x: base.x,
            y: inter_bottom,
            w: base.w,
            h: base_bottom - inter_bottom,
        });
    }
    if i.x > base.x {
        out.push(IntRect {
            x: base.x,
            y: i.y,
            w: i.x - base.x,
            h: i.h,
        });
    }
    let base_right = base.x + base.w;
    let inter_right = i.x + i.w;
    if inter_right < base_right {
        out.push(IntRect {
            x: inter_right,
            y: i.y,
            w: base_right - inter_right,
            h: i.h,
        });
    }
    out.into_iter().filter(|r| r.w > 0 && r.h > 0).collect()
}

fn push_solid(
    out: &mut Vec<SolidColorRenderElement>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: [f32; 4],
) {
    if w <= 0 || h <= 0 {
        return;
    }
    let buffer = SolidColorBuffer::new((w, h), color);
    out.push(SolidColorRenderElement::from_buffer(
        &buffer,
        (x, y),
        smithay::utils::Scale::from(1.0),
        1.0,
        Kind::Unspecified,
    ));
}
