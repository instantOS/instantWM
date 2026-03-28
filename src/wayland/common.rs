//! Shared utilities for all Wayland compositor backends.
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
//! - Render backend trait (`RenderBackend`) for abstracting buffer/cursor operations
//! - Layer shell element counting (`count_upper_layer_render_elements`)

use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::PopupManager;
use smithay::desktop::utils::{send_frames_surface_tree, surface_primary_scanout_output};
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

use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};
use crate::backend::{Backend, WaylandBackendData};
use crate::config::init_config;
use crate::contexts::CoreCtx;
use crate::globals::Globals;
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
    DndIcon {
        icon: WlSurface,
        hotspot: Point<i32, Logical>,
        cursor: Box<CursorPresentation>,
    },
}

/// Resolve effective cursor state shared by nested and DRM backends.
///
/// WM icon overrides have priority over client-provided cursor images.
pub fn resolve_cursor_presentation(
    status: &CursorImageStatus,
    icon_override: Option<CursorIcon>,
    dnd_icon: Option<&WlSurface>,
) -> CursorPresentation {
    let base = if let Some(icon) = icon_override {
        CursorPresentation::Named(icon)
    } else {
        match status {
            CursorImageStatus::Hidden => CursorPresentation::Hidden,
            CursorImageStatus::Named(icon) => CursorPresentation::Named(*icon),
            CursorImageStatus::Surface(surface) => {
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
/// Wayland-specific globals initialization.
///
/// Sets up config, tags, and bar painter font size. This is called before
/// the Wayland compositor is fully initialized, so monitor geometry is not
/// available yet - that will be done via update_geom later.
pub fn init_wayland_globals(g: &mut Globals, wayland: &mut WaylandBackendData) {
    let cfg = init_config();
    g.cfg.screen_width = 1280;
    g.cfg.screen_height = 800;
    crate::globals::apply_config(g, &cfg);
    crate::globals::apply_tags_config(g, &cfg);
    g.cfg.show_bar = true;
    let font_size = wayland_font_size_from_config(&cfg.fonts);
    let font_height = wayland_font_height_from_size(font_size);

    wayland.bar_painter.set_font_size(font_size);

    // CLOSE_BUTTON_WIDTH + CLOSE_BUTTON_DETAIL is the button's visual content;
    // the +2 adds a 1-pixel padding on each side so the button is never flush
    // against the bar edges.
    let min_bar_height = CLOSE_BUTTON_WIDTH + CLOSE_BUTTON_DETAIL + 2;
    // 12 px is a comfortable default vertical padding (≈ 1 line-height * 0.3
    // rounded up) when the user has not explicitly set bar_height in config.
    g.cfg.bar_height = (if cfg.bar_height > 0 {
        font_height + cfg.bar_height
    } else {
        font_height + 12
    })
    .max(min_bar_height);
    g.cfg.horizontal_padding = font_height;

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
pub fn apply_wayland_session_env(socket_name: &str) {
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", socket_name);
        std::env::set_var("XDG_SESSION_TYPE", "wayland");
        std::env::remove_var("DISPLAY");
        std::env::set_var("GDK_BACKEND", "wayland");
        std::env::set_var("QT_QPA_PLATFORM", "wayland");
        std::env::set_var("SDL_VIDEODRIVER", "wayland");
        std::env::set_var("CLUTTER_BACKEND", "wayland");
    }
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
        unsafe { std::env::set_var("DBUS_SESSION_BUS_ADDRESS", addr) };
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
            unsafe { std::env::set_var("DISPLAY", format!(":{}", xwayland.display_number())) };
            let handle_for_wm = loop_handle.clone();
            if let Err(err) = loop_handle.insert_source(xwayland, move |event, _, data| match event
            {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => {
                    data.xdisplay = Some(display_number);
                    unsafe { std::env::set_var("DISPLAY", format!(":{display_number}")) };
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
    if std::env::var("INSTANTWM_WL_AUTOSTART").ok().as_deref() == Some("0") {
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

    let mut core = CoreCtx::new(&mut wm.g, &mut wm.running, &mut wm.bar, &mut wm.focus);

    let bar_buffers = {
        let Backend::Wayland(data) = &mut wm.backend else {
            return Vec::new();
        };

        // Poll systray events
        if let Some(runtime) = data.wayland_systray_runtime.as_mut() {
            let dirty = runtime.poll_events(
                &mut core,
                &mut data.wayland_systray,
                &mut data.wayland_systray_menu,
            );
            if dirty {
                core.bar.mark_dirty();
            }
        }

        crate::bar::wayland::render_bar_buffers(
            &mut core,
            &mut data.bar_painter,
            smithay::utils::Scale::from(1.0),
            &data.wayland_systray,
            data.wayland_systray_menu.as_ref(),
        )
    };
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
    let borders = crate::wayland::render::borders::render_border_elements(&wm.g, state);

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
    let throttle = output.current_mode().and_then(|mode| {
        let refresh = u64::try_from(mode.refresh).ok()?;
        (refresh > 0).then(|| Duration::from_nanos(1_000_000_000_000u64 / refresh))
    });

    for window in state.space.elements() {
        // Only notify windows that are actually visible on this output.
        if let Some(out_geo) = output_geo
            && let Some(win_loc) = state.space.element_location(window)
        {
            let win_size = window.geometry().size;
            let win_rect = smithay::utils::Rectangle::new(win_loc, win_size);
            if !out_geo.overlaps(win_rect) {
                continue;
            }
        }

        if let Some(wl_surface) = window.wl_surface() {
            send_frames_surface_tree(
                &wl_surface,
                output,
                elapsed,
                throttle,
                surface_primary_scanout_output,
            );
            if let Some(toplevel) = window.toplevel() {
                for (popup, _) in PopupManager::popups_for_surface(toplevel.wl_surface()) {
                    send_frames_surface_tree(
                        popup.wl_surface(),
                        output,
                        elapsed,
                        throttle,
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
            throttle,
            surface_primary_scanout_output,
        );
    }
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
