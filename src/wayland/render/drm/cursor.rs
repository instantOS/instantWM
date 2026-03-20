//! Cursor loading and rendering for the standalone DRM/KMS backend.
//!
//! On the DRM backend the compositor must render the cursor itself (there is
//! no host compositor to delegate to).  `CursorManager` pre-loads xcursor
//! theme images for every `CursorIcon` variant at startup and converts them
//! to GPU textures so that `render_element` can composite the right cursor
//! into each frame.

use std::collections::HashMap;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::texture::{TextureBuffer, TextureRenderElement};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::input::pointer::CursorIcon;
use smithay::utils::{Physical, Point, Transform};

use xcursor::CursorTheme;
use xcursor::parser::{Image, parse_xcursor};

use crate::wayland::common::CursorPresentation;

/// A single uploaded cursor frame: texture on the GPU + hotspot in pixels.
struct CursorFrame {
    buffer: TextureBuffer<GlesTexture>,
    hotspot_x: i32,
    hotspot_y: i32,
}

/// Mapping from each `CursorIcon` variant to xcursor theme names to try
/// (in priority order, including legacy fallback names).
const CURSOR_TABLE: &[(CursorIcon, &[&str])] = &[
    (CursorIcon::Default, &["left_ptr", "default", "arrow"]),
    (CursorIcon::ContextMenu, &["context-menu", "left_ptr"]),
    (
        CursorIcon::Help,
        &["help", "question_arrow", "whats_this", "left_ptr"],
    ),
    (
        CursorIcon::Pointer,
        &["pointer", "pointing_hand", "hand2", "hand1", "hand"],
    ),
    (
        CursorIcon::Progress,
        &["progress", "left_ptr_watch", "half-busy", "watch", "wait"],
    ),
    (CursorIcon::Wait, &["wait", "watch", "progress"]),
    (CursorIcon::Cell, &["cell", "plus", "crosshair"]),
    (CursorIcon::Crosshair, &["crosshair", "cross", "cell"]),
    (CursorIcon::Text, &["text", "xterm", "ibeam"]),
    (CursorIcon::VerticalText, &["vertical-text"]),
    (
        CursorIcon::Alias,
        &["alias", "link", "pointing_hand", "hand2", "hand"],
    ),
    (CursorIcon::Copy, &["copy", "dnd-copy"]),
    (CursorIcon::Move, &["move", "fleur", "all-scroll"]),
    (CursorIcon::NoDrop, &["no-drop", "dnd-none"]),
    (
        CursorIcon::NotAllowed,
        &["not-allowed", "crossed_circle", "forbidden"],
    ),
    (CursorIcon::Grab, &["grab", "openhand", "hand1", "hand"]),
    (
        CursorIcon::Grabbing,
        &["grabbing", "closedhand", "fleur", "move"],
    ),
    (
        CursorIcon::EResize,
        &["e-resize", "right_side", "size_hor", "h_double_arrow"],
    ),
    (
        CursorIcon::NResize,
        &["n-resize", "top_side", "size_ver", "v_double_arrow"],
    ),
    (
        CursorIcon::NeResize,
        &[
            "ne-resize",
            "top_right_corner",
            "size_bdiag",
            "fd_double_arrow",
        ],
    ),
    (
        CursorIcon::NwResize,
        &[
            "nw-resize",
            "top_left_corner",
            "size_fdiag",
            "bd_double_arrow",
        ],
    ),
    (
        CursorIcon::SResize,
        &["s-resize", "bottom_side", "size_ver", "v_double_arrow"],
    ),
    (
        CursorIcon::SeResize,
        &[
            "se-resize",
            "bottom_right_corner",
            "size_fdiag",
            "bd_double_arrow",
        ],
    ),
    (
        CursorIcon::SwResize,
        &[
            "sw-resize",
            "bottom_left_corner",
            "size_bdiag",
            "fd_double_arrow",
        ],
    ),
    (
        CursorIcon::WResize,
        &["w-resize", "left_side", "size_hor", "h_double_arrow"],
    ),
    (
        CursorIcon::EwResize,
        &[
            "ew-resize",
            "size_hor",
            "h_double_arrow",
            "sb_h_double_arrow",
        ],
    ),
    (
        CursorIcon::NsResize,
        &[
            "ns-resize",
            "size_ver",
            "v_double_arrow",
            "sb_v_double_arrow",
        ],
    ),
    (
        CursorIcon::NeswResize,
        &["nesw-resize", "size_bdiag", "fd_double_arrow"],
    ),
    (
        CursorIcon::NwseResize,
        &["nwse-resize", "size_fdiag", "bd_double_arrow"],
    ),
    (CursorIcon::ColResize, &["col-resize", "sb_h_double_arrow"]),
    (CursorIcon::RowResize, &["row-resize", "sb_v_double_arrow"]),
    (CursorIcon::AllScroll, &["all-scroll", "fleur", "move"]),
    (CursorIcon::ZoomIn, &["zoom-in"]),
    (CursorIcon::ZoomOut, &["zoom-out"]),
    (CursorIcon::DndAsk, &["dnd-ask", "dnd-none"]),
    (CursorIcon::AllResize, &["all-resize", "fleur", "move"]),
];

/// Manages system cursor textures and decides what to render each frame.
pub struct CursorManager {
    cursors: HashMap<CursorIcon, CursorFrame>,
    default: CursorFrame,
}

impl CursorManager {
    pub fn new(renderer: &mut GlesRenderer, theme: &str, size: u32) -> Self {
        let mut cursors = HashMap::new();
        for &(icon, names) in CURSOR_TABLE {
            if let Some(frame) = load_cursor_names(renderer, theme, size, names) {
                cursors.insert(icon, frame);
            }
        }

        let default = cursors
            .remove(&CursorIcon::Default)
            .unwrap_or_else(|| synthesise_fallback_cursor(renderer));

        Self { cursors, default }
    }

    pub fn render_element(
        &self,
        pointer_location: Point<f64, smithay::utils::Logical>,
        presentation: &CursorPresentation,
    ) -> Option<TextureRenderElement<GlesTexture>> {
        let frame = match presentation {
            CursorPresentation::Hidden | CursorPresentation::Surface { .. } => return None,
            CursorPresentation::Named(icon) => self.frame_for_icon(*icon),
            CursorPresentation::DndIcon { cursor, .. } => {
                return self.render_element(pointer_location, cursor);
            }
        };

        Some(frame_to_element(frame, pointer_location))
    }

    fn frame_for_icon(&self, icon: CursorIcon) -> &CursorFrame {
        self.cursors.get(&icon).unwrap_or(&self.default)
    }
}

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
        if theme != "default"
            && let Some(frame) = load_cursor_frame(renderer, "default", name, size)
        {
            return Some(frame);
        }
    }
    None
}

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

    let best_size = images
        .iter()
        .map(|img| img.size)
        .min_by_key(|&s| (s as i64 - size as i64).unsigned_abs())?;

    let frames: Vec<&Image> = images.iter().filter(|img| img.size == best_size).collect();
    let frame = frames.first()?;

    let buf = import_image(renderer, frame)?;
    Some(CursorFrame {
        buffer: buf,
        hotspot_x: frame.xhot as i32,
        hotspot_y: frame.yhot as i32,
    })
}

fn import_image(renderer: &mut GlesRenderer, image: &Image) -> Option<TextureBuffer<GlesTexture>> {
    TextureBuffer::from_memory(
        renderer,
        &image.pixels_rgba,
        Fourcc::Abgr8888,
        (image.width as i32, image.height as i32),
        false,
        1,
        Transform::Normal,
        None,
    )
    .ok()
}

fn synthesise_fallback_cursor(renderer: &mut GlesRenderer) -> CursorFrame {
    const W: usize = 8;
    const H: usize = 8;
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

fn frame_to_element(
    frame: &CursorFrame,
    pointer_location: Point<f64, smithay::utils::Logical>,
) -> TextureRenderElement<GlesTexture> {
    let render_x = pointer_location.x - frame.hotspot_x as f64;
    let render_y = pointer_location.y - frame.hotspot_y as f64;

    let pos: Point<f64, Physical> = Point::from((render_x, render_y));

    TextureRenderElement::from_texture_buffer(pos, &frame.buffer, None, None, None, Kind::Cursor)
}
