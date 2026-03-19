//! DRM/KMS rendering and GPU output management.

use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, GbmBufferedSurface};
use smithay::backend::renderer::Bind;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::drm::control::Device as ControlDevice;
use smithay::reexports::drm::control::connector;
use smithay::reexports::drm::control::crtc;
use smithay::utils::{Physical, Point, Rectangle};

use crate::backend::wayland::compositor::WaylandState;
use crate::wayland::common::{
    CursorPresentation, build_common_scene_elements, count_upper_layer_render_elements,
    get_render_element_counts, resolve_cursor_presentation, send_frame_callbacks,
};
use crate::wm::Wm;

// Re-export cursor management
pub use cursor::CursorManager;
pub use state::{
    CURSOR_SIZE, DEFAULT_SCREEN_HEIGHT, DEFAULT_SCREEN_WIDTH, OutputHitRegion, OutputSurfaceEntry,
    SharedDrmState, sync_monitors_from_outputs_vec,
};

pub mod cursor {
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
        (CursorIcon::ContextMenu, &["context-menu"]),
        (CursorIcon::Help, &["help", "question_arrow", "whats_this"]),
        (CursorIcon::Pointer, &["pointer", "hand2", "hand1", "hand"]),
        (
            CursorIcon::Progress,
            &["progress", "left_ptr_watch", "half-busy"],
        ),
        (CursorIcon::Wait, &["wait", "watch"]),
        (CursorIcon::Cell, &["cell", "plus"]),
        (CursorIcon::Crosshair, &["crosshair", "cross"]),
        (CursorIcon::Text, &["text", "xterm", "ibeam"]),
        (CursorIcon::VerticalText, &["vertical-text"]),
        (CursorIcon::Alias, &["alias", "link"]),
        (CursorIcon::Copy, &["copy", "dnd-copy"]),
        (CursorIcon::Move, &["move", "fleur"]),
        (CursorIcon::NoDrop, &["no-drop", "dnd-none"]),
        (
            CursorIcon::NotAllowed,
            &["not-allowed", "crossed_circle", "forbidden"],
        ),
        (CursorIcon::Grab, &["grab", "openhand", "hand1"]),
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
            if theme != "default" {
                if let Some(frame) = load_cursor_frame(renderer, "default", name, size) {
                    return Some(frame);
                }
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

    fn import_image(
        renderer: &mut GlesRenderer,
        image: &Image,
    ) -> Option<TextureBuffer<GlesTexture>> {
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

        TextureRenderElement::from_texture_buffer(
            pos,
            &frame.buffer,
            None,
            None,
            None,
            Kind::Cursor,
        )
    }
}

pub mod state {
    //! Shared state for the DRM backend.

    use std::collections::{HashMap, HashSet};

    use smithay::backend::allocator::gbm::GbmAllocator;
    use smithay::backend::drm::{DrmDeviceFd, GbmBufferedSurface};
    use smithay::backend::renderer::damage::OutputDamageTracker;
    use smithay::output::Output;
    use smithay::reexports::drm::control::crtc;

    use crate::types::Rect;
    use crate::wm::Wm;

    pub const DEFAULT_SCREEN_WIDTH: i32 = 1280;
    pub const DEFAULT_SCREEN_HEIGHT: i32 = 800;
    pub const CURSOR_SIZE: u32 = 24;

    pub struct OutputHitRegion {
        pub crtc: crtc::Handle,
        pub x_offset: i32,
        pub width: i32,
    }

    pub struct OutputSurfaceEntry {
        pub crtc: crtc::Handle,
        pub surface: GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>,
        pub output: Output,
        pub damage_tracker: OutputDamageTracker,
        pub x_offset: i32,
        pub width: i32,
        pub height: i32,
    }

    pub struct SharedDrmState {
        pub session_active: bool,
        pub render_flags: HashMap<crtc::Handle, bool>,
        pub total_width: i32,
        pub total_height: i32,
        pub completed_crtcs: Vec<crtc::Handle>,
        pub pending_crtcs: HashSet<crtc::Handle>,
        pub output_hit_regions: Vec<OutputHitRegion>,
    }

    impl SharedDrmState {
        pub fn new(total_width: i32, total_height: i32) -> Self {
            Self {
                session_active: true,
                render_flags: HashMap::new(),
                total_width,
                total_height,
                completed_crtcs: Vec::new(),
                pending_crtcs: HashSet::new(),
                output_hit_regions: Vec::new(),
            }
        }

        pub fn mark_all_dirty(&mut self) {
            for flag in self.render_flags.values_mut() {
                *flag = true;
            }
        }

        pub fn mark_dirty(&mut self, crtc: crtc::Handle) {
            if let Some(flag) = self.render_flags.get_mut(&crtc) {
                *flag = true;
            }
        }

        pub fn mark_pointer_output_dirty(&mut self, px: i32) {
            for entry in &self.output_hit_regions {
                if px >= entry.x_offset && px < entry.x_offset + entry.width {
                    self.mark_dirty(entry.crtc);
                    return;
                }
            }
            self.mark_all_dirty();
        }
    }

    pub fn sync_monitors_from_outputs_vec(wm: &mut Wm, surfaces: &[super::OutputSurfaceEntry]) {
        wm.g.monitors.clear();
        let tag_template = wm.g.cfg.tag_template.clone();

        for (i, surface) in surfaces.iter().enumerate() {
            let x = surface.x_offset;
            let y = 0i32;
            let w = surface.width;
            let h = surface.height;

            let mut mon = crate::types::Monitor::new_with_values(
                wm.g.cfg.mfact,
                wm.g.cfg.nmaster,
                wm.g.cfg.show_bar,
                wm.g.cfg.top_bar,
            );
            mon.num = i as i32;
            mon.monitor_rect = Rect { x, y, w, h };
            mon.work_rect = Rect { x, y, w, h };
            mon.current_tag = 1;
            mon.prev_tag = 1;
            mon.tag_set = [1, 1];
            mon.init_tags(&tag_template);
            mon.update_bar_position(wm.g.cfg.bar_height);
            wm.g.monitors.push(mon);
        }

        wm.g.cfg.screen_width = surfaces
            .iter()
            .map(|s| s.x_offset + s.width)
            .max()
            .unwrap_or(DEFAULT_SCREEN_WIDTH);
        wm.g.cfg.screen_height = surfaces
            .iter()
            .map(|s| s.height)
            .max()
            .unwrap_or(DEFAULT_SCREEN_HEIGHT);

        if wm.g.monitors.is_empty() {
            let mut mon = crate::types::Monitor::new_with_values(
                wm.g.cfg.mfact,
                wm.g.cfg.nmaster,
                wm.g.cfg.show_bar,
                wm.g.cfg.top_bar,
            );
            mon.monitor_rect = Rect {
                x: 0,
                y: 0,
                w: DEFAULT_SCREEN_WIDTH,
                h: DEFAULT_SCREEN_HEIGHT,
            };
            mon.work_rect = Rect {
                x: 0,
                y: 0,
                w: DEFAULT_SCREEN_WIDTH,
                h: DEFAULT_SCREEN_HEIGHT,
            };
            mon.init_tags(&tag_template);
            mon.update_bar_position(wm.g.cfg.bar_height);
            wm.g.monitors.push(mon);
        }

        for (i, mon) in wm.g.monitors.iter_mut() {
            mon.num = i as i32;
        }

        if wm.g.selected_monitor_id() >= wm.g.monitors.count() {
            wm.g.set_selected_monitor(0);
        }
    }
}

render_elements! {
    pub DrmExtras<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Cursor=TextureRenderElement<GlesTexture>,
    Space=smithay::desktop::space::SpaceRenderElements<GlesRenderer, smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<GlesRenderer>>,
}

pub fn build_output_surfaces(
    drm_device: &mut DrmDevice,
    renderer: &mut GlesRenderer,
    state: &WaylandState,
    gbm_device: &GbmDevice<DrmDeviceFd>,
) -> Vec<OutputSurfaceEntry> {
    let gbm_allocator = GbmAllocator::new(
        gbm_device.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let color_formats: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Xrgb8888];
    let renderer_formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();

    let mut output_surfaces: Vec<OutputSurfaceEntry> = Vec::new();
    let mut output_x_offset: i32 = 0;

    let res = drm_device.resource_handles().expect("drm resource_handles");
    let mut used_crtcs: Vec<crtc::Handle> = Vec::new();

    for &conn_handle in res.connectors() {
        let Ok(conn_info) = drm_device.get_connector(conn_handle, false) else {
            continue;
        };
        if conn_info.state() != connector::State::Connected
            && conn_info.state() != connector::State::Unknown
        {
            continue;
        }
        let modes = conn_info.modes();
        if modes.is_empty() {
            continue;
        }

        let mut sorted_modes = modes.to_vec();
        sorted_modes.sort_by(|a, b| {
            let (aw, ah) = a.size();
            let (bw, bh) = b.size();
            (bw as u64 * bh as u64)
                .cmp(&(aw as u64 * ah as u64))
                .then_with(|| b.vrefresh().cmp(&a.vrefresh()))
        });
        let mode = sorted_modes[0];

        let encoder_crtcs: Vec<crtc::Handle> = conn_info
            .encoders()
            .iter()
            .filter_map(|&enc_h| drm_device.get_encoder(enc_h).ok())
            .flat_map(|enc| res.filter_crtcs(enc.possible_crtcs()))
            .collect();

        let Some(&picked_crtc) = encoder_crtcs.iter().find(|c| !used_crtcs.contains(c)) else {
            continue;
        };
        used_crtcs.push(picked_crtc);

        let drm_surface = drm_device
            .create_surface(picked_crtc, mode, &[conn_handle])
            .expect("create_surface");
        let gbm_surface = GbmBufferedSurface::new(
            drm_surface,
            gbm_allocator.clone(),
            color_formats,
            renderer_formats.iter().cloned(),
        )
        .expect("GbmBufferedSurface::new");

        let (mode_w, mode_h) = mode.size();
        let (mode_w, mode_h) = (mode_w as i32, mode_h as i32);
        let output_name = format!(
            "{}-{}",
            connector_type_name(conn_info.interface()),
            conn_info.interface_id()
        );
        log::info!(
            "Output {output_name}: {mode_w}x{mode_h}@{}Hz on CRTC {:?}",
            mode.vrefresh(),
            picked_crtc
        );

        let output = Output::new(
            output_name,
            PhysicalProperties {
                size: {
                    let (mm_w, mm_h) = conn_info.size().unwrap_or((0, 0));
                    (mm_w as i32, mm_h as i32).into()
                },
                subpixel: Subpixel::Unknown,
                make: "instantOS".into(),
                model: "instantWM".into(),
            },
        );
        let out_mode = OutputMode {
            size: (mode_w, mode_h).into(),
            refresh: (mode.vrefresh() as i32) * 1000,
        };
        output.change_current_state(
            Some(out_mode),
            Some(smithay::utils::Transform::Normal),
            Some(Scale::Integer(1)),
            Some((output_x_offset, 0).into()),
        );
        output.set_preferred(out_mode);
        let _global = output.create_global::<WaylandState>(&state.display_handle);

        let damage_tracker = OutputDamageTracker::from_output(&output);

        output_surfaces.push(OutputSurfaceEntry {
            crtc: picked_crtc,
            surface: gbm_surface,
            output: output.clone(),
            damage_tracker,
            x_offset: output_x_offset,
            width: mode_w,
            height: mode_h,
        });
        output_x_offset += mode_w;
    }

    output_surfaces
}

fn connector_type_name(interface: connector::Interface) -> &'static str {
    match interface {
        connector::Interface::DVII => "DVI-I",
        connector::Interface::DVID => "DVI-D",
        connector::Interface::DVIA => "DVI-A",
        connector::Interface::SVideo => "S-Video",
        connector::Interface::DisplayPort => "DP",
        connector::Interface::HDMIA => "HDMI-A",
        connector::Interface::HDMIB => "HDMI-B",
        connector::Interface::EmbeddedDisplayPort => "eDP",
        connector::Interface::VGA => "VGA",
        connector::Interface::LVDS => "LVDS",
        connector::Interface::DSI => "DSI",
        connector::Interface::DPI => "DPI",
        connector::Interface::Composite => "Composite",
        _ => "Unknown",
    }
}

pub fn render_drm_output(
    wm: &mut Wm,
    state: &mut WaylandState,
    renderer: &mut GlesRenderer,
    entry: &mut OutputSurfaceEntry,
    cursor_manager: &CursorManager,
    pointer_location: Point<f64, smithay::utils::Logical>,
    start_time: std::time::Instant,
) -> bool {
    let (dmabuf, age) = match entry.surface.next_buffer() {
        Ok(buf) => buf,
        Err(e) => {
            log::trace!("next_buffer: {e}");
            return false;
        }
    };

    let mut dmabuf_clone = dmabuf.clone();
    let Ok(mut target) = renderer.bind(&mut dmabuf_clone) else {
        log::warn!("renderer bind failed");
        return false;
    };

    let local_pointer = Point::from((
        pointer_location.x - entry.x_offset as f64,
        pointer_location.y,
    ));
    let cursor_presentation = resolve_cursor_presentation(
        &state.cursor_image_status,
        state.cursor_icon_override,
        state.dnd_icon.as_ref(),
    );

    let cursor_elements: Vec<DrmExtras> = build_cursor_elements(
        renderer,
        cursor_manager,
        &cursor_presentation,
        local_pointer,
    );

    let scene = build_common_scene_elements(wm, state, renderer, entry.x_offset);
    let space_render_elements = smithay::desktop::space::space_render_elements(
        renderer,
        [&state.space],
        &entry.output,
        1.0,
    )
    .expect("space render elements");

    // Shared: count upper layer elements
    let num_upper = count_upper_layer_render_elements(renderer, &entry.output);

    // Shared: get element counts for pre-allocation (include cursor elements)
    let counts = get_render_element_counts(&scene, space_render_elements.len(), num_upper);
    let mut render_elements = Vec::with_capacity(counts.total() + cursor_elements.len());

    // Backend-specific: cursor elements come first in DRM (winit handles cursor differently)
    for elem in cursor_elements {
        render_elements.push(elem);
    }

    // Shared: assemble remaining elements in z-order
    super::assemble_scene_elements!(
        DrmExtras,
        scene,
        space_render_elements,
        num_upper,
        render_elements
    );

    let render_result = entry.damage_tracker.render_output(
        renderer,
        &mut target,
        age as usize,
        &render_elements,
        [0.05, 0.05, 0.07, 1.0],
    );

    crate::backend::wayland::compositor::screencopy::submit_pending_screencopies(
        &mut state.pending_screencopies,
        renderer,
        &target,
        &entry.output,
        start_time,
    );
    drop(target);

    match render_result {
        Ok(result) => {
            let damage: Option<Vec<Rectangle<i32, Physical>>> = result.damage.cloned();
            if let Err(e) = entry.surface.queue_buffer(None, damage, ()) {
                log::warn!("queue_buffer: {e}");
                return false;
            }
        }
        Err(e) => {
            log::warn!("render_output: {:?}", e);
            return false;
        }
    }

    send_frame_callbacks(state, &entry.output, start_time.elapsed());
    true
}

fn build_cursor_elements(
    renderer: &mut GlesRenderer,
    cursor_manager: &CursorManager,
    cursor_presentation: &CursorPresentation,
    local_pointer: Point<f64, smithay::utils::Logical>,
) -> Vec<DrmExtras> {
    let mut custom_elements = Vec::new();

    match cursor_presentation {
        CursorPresentation::Hidden => {}
        CursorPresentation::Named(_) => {
            if let Some(cursor_elem) =
                cursor_manager.render_element(local_pointer, cursor_presentation)
            {
                custom_elements.push(DrmExtras::Cursor(cursor_elem));
            }
        }
        CursorPresentation::Surface { surface, hotspot } => {
            // Double-check that the surface is still alive before rendering.
            if !smithay::utils::IsAlive::alive(surface) {
                return custom_elements;
            }
            let cursor_loc = smithay::utils::Point::<i32, smithay::utils::Physical>::from((
                (local_pointer.x - hotspot.x as f64).round() as i32,
                (local_pointer.y - hotspot.y as f64).round() as i32,
            ));
            let cursor_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                    renderer,
                    surface,
                    cursor_loc,
                    smithay::utils::Scale::from(1.0),
                    1.0,
                    smithay::backend::renderer::element::Kind::Cursor,
                );
            for elem in cursor_elements {
                custom_elements.push(DrmExtras::Surface(elem));
            }
        }
        CursorPresentation::DndIcon {
            icon,
            hotspot,
            cursor,
        } => {
            // Render the base cursor first
            custom_elements.extend(build_cursor_elements(
                renderer,
                cursor_manager,
                cursor,
                local_pointer,
            ));

            // Double-check that the drag icon surface is still alive before rendering.
            if !smithay::utils::IsAlive::alive(icon) {
                return custom_elements;
            }

            // Then render the drag icon
            let dnd_loc = smithay::utils::Point::<i32, smithay::utils::Physical>::from((
                (local_pointer.x - hotspot.x as f64).round() as i32,
                (local_pointer.y - hotspot.y as f64).round() as i32,
            ));
            let dnd_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                    renderer,
                    icon,
                    dnd_loc,
                    smithay::utils::Scale::from(1.0),
                    1.0,
                    smithay::backend::renderer::element::Kind::Cursor,
                );
            for elem in dnd_elements {
                custom_elements.push(DrmExtras::Surface(elem));
            }
        }
    }

    custom_elements
}
