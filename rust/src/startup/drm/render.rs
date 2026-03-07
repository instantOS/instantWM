use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::Bind;
use smithay::desktop::space::render_output;
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
    let mut custom_elements: Vec<DrmExtras> = cursor_elements;
    for elem in scene.overlays {
        custom_elements.push(DrmExtras::Surface(elem));
    }
    for elem in scene.bar {
        custom_elements.push(DrmExtras::Memory(elem));
    }
    for elem in scene.borders {
        custom_elements.push(DrmExtras::Solid(elem));
    }

    let render_result = render_output(
        &entry.output,
        renderer,
        &mut target,
        1.0,
        age as usize,
        [&state.space],
        &custom_elements,
        &mut entry.damage_tracker,
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
