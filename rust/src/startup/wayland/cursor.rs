//! Cursor loading and rendering for the standalone DRM/KMS backend.
//!
//! In the nested (winit) backend the host compositor handles cursor
//! rendering entirely — we just call `window().set_cursor(icon)`.  In
//! standalone DRM mode we must render the cursor ourselves as a render
//! element on top of every frame.
//!
//! # How it works
//!
//! 1. At startup, `CursorManager::new()` loads one or more named cursor
//!    images from the system xcursor theme (defaulting to "default") and
//!    uploads them to the GPU as `TextureBuffer<GlesTexture>` objects.
//! 2. Each frame, `CursorManager::render_element()` returns a
//!    `TextureRenderElement` positioned at the current pointer location
//!    and offset by the cursor hotspot.
//! 3. The caller is responsible for prepending the element to the custom
//!    element list that is passed to `render_output`.
//!
//! # Cursor image source priority
//!
//! 1. `CursorImageStatus::Hidden`    → no element returned (cursor hidden).
//! 2. `CursorImageStatus::Named(icon)` → use the preloaded system texture
//!    that best matches the `CursorIcon` name.
//! 3. `CursorImageStatus::Surface(_)` → we currently do not render client
//!    cursor surfaces directly on DRM, so we fall back to the default cursor
//!    texture instead of hiding it.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::texture::{TextureBuffer, TextureRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::input::pointer::{CursorIcon, CursorImageStatus};
use smithay::utils::{Physical, Point, Transform};

use xcursor::parser::{parse_xcursor, Image};
use xcursor::CursorTheme;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single uploaded cursor frame: texture on the GPU + hotspot in pixels.
struct CursorFrame {
    buffer: TextureBuffer<GlesTexture>,
    hotspot_x: i32,
    hotspot_y: i32,
}

/// Manages system cursor textures and decides what to render each frame.
///
/// Create one instance at compositor startup and keep it alive for the
/// duration of the event loop.
pub struct CursorManager {
    /// The "default" / arrow cursor, used as the fallback.
    default: CursorFrame,
    /// "move" cursor for title-bar drag operations.
    move_cursor: Option<CursorFrame>,
    /// Resize cursors, indexed by the four diagonal / orthogonal directions.
    resize_nw: Option<CursorFrame>,
    resize_ne: Option<CursorFrame>,
    resize_sw: Option<CursorFrame>,
    resize_se: Option<CursorFrame>,
    resize_n: Option<CursorFrame>,
    resize_s: Option<CursorFrame>,
    resize_e: Option<CursorFrame>,
    resize_w: Option<CursorFrame>,
}

impl CursorManager {
    /// Load cursor textures from the system xcursor theme and upload them to
    /// `renderer`.
    ///
    /// `theme` is the theme name to try first (e.g. `"Adwaita"`, `"default"`).
    /// If a particular cursor name is not found in `theme` the loader
    /// automatically falls through to the `"default"` theme.  If the theme
    /// cannot be found at all we synthesise a plain white 8×8 square so the
    /// compositor can still start.
    pub fn new(renderer: &mut GlesRenderer, theme: &str, size: u32) -> Self {
        // The arrow cursor is mandatory; synthesise one if nothing is found.
        let default = load_cursor_names(renderer, theme, size, &["left_ptr", "default", "arrow"])
            .unwrap_or_else(|| synthesise_fallback_cursor(renderer));

        Self {
            default,
            move_cursor: load_cursor_names(renderer, theme, size, &["grabbing", "fleur", "move"]),
            resize_nw: load_cursor_names(
                renderer,
                theme,
                size,
                &["nw-resize", "size_fdiag", "bd_double_arrow"],
            ),
            resize_ne: load_cursor_names(
                renderer,
                theme,
                size,
                &["ne-resize", "size_bdiag", "fd_double_arrow"],
            ),
            resize_sw: load_cursor_names(
                renderer,
                theme,
                size,
                &["sw-resize", "size_bdiag", "fd_double_arrow"],
            ),
            resize_se: load_cursor_names(
                renderer,
                theme,
                size,
                &["se-resize", "size_fdiag", "bd_double_arrow"],
            ),
            resize_n: load_cursor_names(
                renderer,
                theme,
                size,
                &["n-resize", "size_ver", "v_double_arrow"],
            ),
            resize_s: load_cursor_names(
                renderer,
                theme,
                size,
                &["s-resize", "size_ver", "v_double_arrow"],
            ),
            resize_e: load_cursor_names(
                renderer,
                theme,
                size,
                &["e-resize", "size_hor", "h_double_arrow"],
            ),
            resize_w: load_cursor_names(
                renderer,
                theme,
                size,
                &["w-resize", "size_hor", "h_double_arrow"],
            ),
        }
    }

    /// Return the cursor render element to be drawn at `pointer_location` for
    /// this frame, or `None` if the cursor should be hidden.
    ///
    /// - `pointer_location`: pointer position in logical (compositor) pixels.
    /// - `status`: the `CursorImageStatus` stored in `WaylandState`.
    /// - `icon_override`: an optional `CursorIcon` set by the WM itself (e.g.
    ///   during a resize drag) that takes priority over `status`.
    pub fn render_element(
        &self,
        pointer_location: Point<f64, smithay::utils::Logical>,
        status: &CursorImageStatus,
        icon_override: Option<CursorIcon>,
    ) -> Option<TextureRenderElement<GlesTexture>> {
        // WM-set icon override wins over everything except Hidden.
        let frame = if let Some(icon) = icon_override {
            self.frame_for_icon(icon)
        } else {
            match status {
                CursorImageStatus::Hidden => return None,
                CursorImageStatus::Named(icon) => self.frame_for_icon(*icon),
                // Client-provided cursor surfaces are not rendered explicitly
                // on DRM yet; keep the pointer visible with a default arrow.
                CursorImageStatus::Surface(_) => &self.default,
            }
        };

        Some(frame_to_element(frame, pointer_location))
    }

    /// Pick the most appropriate preloaded `CursorFrame` for a given
    /// `CursorIcon`.  Falls back to `self.default` for any icon we don't have
    /// a dedicated texture for.
    fn frame_for_icon(&self, icon: CursorIcon) -> &CursorFrame {
        match icon {
            CursorIcon::Grabbing | CursorIcon::AllScroll => {
                self.move_cursor.as_ref().unwrap_or(&self.default)
            }
            CursorIcon::NwResize | CursorIcon::SeResize | CursorIcon::NwseResize => self
                .resize_nw
                .as_ref()
                .or(self.resize_se.as_ref())
                .unwrap_or(&self.default),
            CursorIcon::NeResize | CursorIcon::SwResize | CursorIcon::NeswResize => self
                .resize_ne
                .as_ref()
                .or(self.resize_sw.as_ref())
                .unwrap_or(&self.default),
            CursorIcon::NResize | CursorIcon::SResize | CursorIcon::NsResize => self
                .resize_n
                .as_ref()
                .or(self.resize_s.as_ref())
                .unwrap_or(&self.default),
            CursorIcon::EResize | CursorIcon::WResize | CursorIcon::EwResize => self
                .resize_e
                .as_ref()
                .or(self.resize_w.as_ref())
                .unwrap_or(&self.default),
            _ => &self.default,
        }
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Try each name in `names` against `theme` (and then the `"default"` theme
/// as a fallback) and return the first one that loads successfully.
fn load_cursor_names(
    renderer: &mut GlesRenderer,
    theme: &str,
    size: u32,
    names: &[&str],
) -> Option<CursorFrame> {
    for name in names {
        if let Some(frame) = load_cursor_frame(renderer, theme, name, size) {
            return Some(frame);
        }
        if theme != "default" {
            if let Some(frame) = load_cursor_frame(renderer, "default", name, size) {
                return Some(frame);
            }
        }
    }
    None
}

/// Try to load a cursor by name from the given theme at the given nominal
/// size.  Returns the frame closest to `size` if multiple sizes are present.
fn load_cursor_frame(
    renderer: &mut GlesRenderer,
    theme_name: &str,
    cursor_name: &str,
    size: u32,
) -> Option<CursorFrame> {
    let theme = CursorTheme::load(theme_name);
    let path = theme.load_icon(cursor_name)?;
    let bytes = std::fs::read(&path).ok()?;
    let images = parse_xcursor(&bytes)?;
    if images.is_empty() {
        return None;
    }

    // Pick the size closest to the requested nominal size.
    let best_size = images
        .iter()
        .map(|img| img.size)
        .min_by_key(|&s| (s as i64 - size as i64).unsigned_abs())?;

    let frames: Vec<&Image> = images.iter().filter(|img| img.size == best_size).collect();
    // Use the first frame (index 0).  Animated cursors are not supported yet.
    let frame = frames.first()?;

    let buf = import_image(renderer, frame)?;
    Some(CursorFrame {
        buffer: buf,
        hotspot_x: frame.xhot as i32,
        hotspot_y: frame.yhot as i32,
    })
}

/// Upload one xcursor `Image` (RGBA bytes) to the GPU as a `TextureBuffer`.
fn import_image(renderer: &mut GlesRenderer, image: &Image) -> Option<TextureBuffer<GlesTexture>> {
    // xcursor gives us RGBA bytes.  In DRM fourcc notation on little-endian
    // hardware, RGBA memory layout = Fourcc::Abgr8888.
    TextureBuffer::from_memory(
        renderer,
        &image.pixels_rgba,
        Fourcc::Abgr8888,
        (image.width as i32, image.height as i32),
        false, // not y-flipped
        1,     // scale factor 1 (cursor textures are always physical pixels)
        Transform::Normal,
        None, // no opaque region hint (cursor has transparency)
    )
    .ok()
}

/// Create a minimal 8×8 solid-white cursor as a last-resort fallback so the
/// compositor can always start even when no cursor theme is installed.
fn synthesise_fallback_cursor(renderer: &mut GlesRenderer) -> CursorFrame {
    const W: usize = 8;
    const H: usize = 8;
    // RGBA: fully opaque white pixels
    let pixels: Vec<u8> = vec![0xFF; W * H * 4];
    let buffer = TextureBuffer::from_memory(
        renderer,
        &pixels,
        Fourcc::Abgr8888,
        (W as i32, H as i32),
        false,
        1,
        Transform::Normal,
        None,
    )
    .expect("synthesise_fallback_cursor: from_memory failed");

    CursorFrame {
        buffer,
        hotspot_x: 0,
        hotspot_y: 0,
    }
}

/// Build a `TextureRenderElement` for a `CursorFrame` at the given logical
/// pointer position, offset by the cursor hotspot.
fn frame_to_element(
    frame: &CursorFrame,
    pointer_location: Point<f64, smithay::utils::Logical>,
) -> TextureRenderElement<GlesTexture> {
    // The render position is in physical (integer) output coordinates.  Since
    // we always use scale = 1 in the DRM backend for now we can treat logical
    // and physical coordinates identically.
    let render_x = pointer_location.x - frame.hotspot_x as f64;
    let render_y = pointer_location.y - frame.hotspot_y as f64;

    let pos: Point<f64, Physical> = Point::from((render_x, render_y));

    TextureRenderElement::from_texture_buffer(
        pos,
        &frame.buffer,
        None, // alpha = 1.0
        None, // src crop (None = whole texture)
        None, // override size (None = texture's own size)
        // Keep cursor in normal composition so z-order matches element order
        // (cursor should remain above borders and all window content).
        Kind::Unspecified,
    )
}
