use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::Bind;
use smithay::utils::{Physical, Point, Rectangle};

use crate::backend::wayland::compositor::WaylandState;
use crate::startup::common_wayland::{
    build_common_scene_elements, resolve_cursor_presentation, send_frame_callbacks,
    CursorPresentation,
};
use crate::startup::wayland::cursor::CursorManager;
use crate::wm::Wm;

use super::state::OutputSurfaceEntry;

render_elements! {
    pub DrmExtras<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Cursor=TextureRenderElement<GlesTexture>,
    Space=smithay::desktop::space::SpaceRenderElements<GlesRenderer, smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<GlesRenderer>>,
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
