//! DRM/KMS rendering and GPU output management.

use std::collections::{HashMap, HashSet};

use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, GbmBufferedSurface};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::texture::{TextureBuffer, TextureRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::Bind;
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::drm::control::connector;
use smithay::reexports::drm::control::crtc;
use smithay::reexports::drm::control::Device as ControlDevice;
use smithay::utils::{Physical, Point, Rectangle};

use crate::backend::wayland::compositor::WaylandState;
use crate::types::Rect;
use crate::wayland::common::{
    build_common_scene_elements, resolve_cursor_presentation, send_frame_callbacks,
    CursorPresentation,
};
use crate::wm::Wm;

// Re-export cursor management
pub use cursor::CursorManager;
pub use state::{
    sync_monitors_from_outputs_vec, OutputHitRegion, OutputSurfaceEntry, SharedDrmState,
    CURSOR_SIZE, DEFAULT_SCREEN_HEIGHT, DEFAULT_SCREEN_WIDTH,
};

pub mod cursor {
    //! Cursor loading and rendering for the standalone DRM/KMS backend.

    use smithay::backend::allocator::Fourcc;
    use smithay::backend::renderer::element::texture::{TextureBuffer, TextureRenderElement};
    use smithay::backend::renderer::element::Kind;
    use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
    use smithay::input::pointer::CursorIcon;
    use smithay::utils::{Physical, Point, Transform};

    use xcursor::parser::{parse_xcursor, Image};
    use xcursor::CursorTheme;

    use crate::wayland::common::CursorPresentation;

    /// A single uploaded cursor frame: texture on the GPU + hotspot in pixels.
    struct CursorFrame {
        buffer: TextureBuffer<GlesTexture>,
        hotspot_x: i32,
        hotspot_y: i32,
    }

    /// Manages system cursor textures and decides what to render each frame.
    pub struct CursorManager {
        default: CursorFrame,
        move_cursor: Option<CursorFrame>,
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
        pub fn new(renderer: &mut GlesRenderer, theme: &str, size: u32) -> Self {
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

        pub fn render_element(
            &self,
            pointer_location: Point<f64, smithay::utils::Logical>,
            presentation: &CursorPresentation,
        ) -> Option<TextureRenderElement<GlesTexture>> {
            let frame = match presentation {
                CursorPresentation::Hidden | CursorPresentation::Surface { .. } => return None,
                CursorPresentation::Named(icon) => self.frame_for_icon(*icon),
            };

            Some(frame_to_element(frame, pointer_location))
        }

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
    use smithay::utils::Point;

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
        pub pointer_location: Point<f64, smithay::utils::Logical>,
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
                pointer_location: Point::from(((total_width / 2) as f64, (total_height / 2) as f64)),
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
    let cursor_presentation =
        resolve_cursor_presentation(&state.cursor_image_status, state.cursor_icon_override);

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

    let num_upper = count_upper_layer_render_elements(renderer, &entry.output);

    let mut render_elements = Vec::with_capacity(
        cursor_elements.len()
            + scene.overlays.len()
            + scene.bar.len()
            + scene.borders.len()
            + space_render_elements.len(),
    );

    for elem in cursor_elements {
        render_elements.push(elem);
    }
    for elem in scene.overlays {
        render_elements.push(DrmExtras::Surface(elem));
    }

    let mut space_iter = space_render_elements.into_iter();
    for elem in space_iter.by_ref().take(num_upper) {
        render_elements.push(DrmExtras::Space(elem));
    }

    for elem in scene.bar {
        render_elements.push(DrmExtras::Memory(elem));
    }
    for elem in scene.borders {
        render_elements.push(DrmExtras::Solid(elem));
    }
    for elem in space_iter {
        render_elements.push(DrmExtras::Space(elem));
    }

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

fn count_upper_layer_render_elements(
    renderer: &mut GlesRenderer,
    output: &smithay::output::Output,
) -> usize {
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

fn build_cursor_elements(
    renderer: &mut GlesRenderer,
    cursor_manager: &CursorManager,
    cursor_presentation: &CursorPresentation,
    local_pointer: Point<f64, smithay::utils::Logical>,
) -> Vec<DrmExtras> {
    let mut custom_elements = Vec::new();

    if let CursorPresentation::Surface { surface, hotspot } = cursor_presentation {
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

    if let Some(cursor_elem) = cursor_manager.render_element(local_pointer, cursor_presentation) {
        custom_elements.push(DrmExtras::Cursor(cursor_elem));
    }

    custom_elements
}
