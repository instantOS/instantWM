use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::WinitGraphicsBackend;
use smithay::output::Output;

use crate::backend::wayland::compositor::WaylandState;
use crate::wayland::common::{
    build_common_scene_elements, count_upper_layer_render_elements, get_render_element_counts,
    resolve_cursor_presentation, send_frame_callbacks, CursorPresentation,
};
use crate::wm::Wm;

render_elements! {
    pub WaylandExtras<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Space=smithay::desktop::space::SpaceRenderElements<GlesRenderer, smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<GlesRenderer>>,
}

/// Render a frame using the winit backend.
pub fn render_frame(
    wm: &mut Wm,
    state: &mut WaylandState,
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
    start_time: std::time::Instant,
) {
    // Backend-specific: apply cursor via window API
    let cursor_presentation = resolve_cursor_presentation(
        &state.cursor_image_status,
        state.cursor_icon_override,
        state.dnd_icon.as_ref(),
    );
    apply_cursor_presentation_internal(backend, &cursor_presentation);
    state.tick_window_animations();

    // Backend-specific: get buffer age
    let buffer_age = backend.buffer_age().unwrap_or(0);

    // Backend-specific: bind to get framebuffer
    let (renderer, mut framebuffer) = backend.bind().expect("renderer bind");

    // Shared: build scene elements
    let scene = build_common_scene_elements(wm, state, renderer, 0);

    // Shared: get space render elements
    let space_render_elements =
        smithay::desktop::space::space_render_elements(renderer, [&state.space], output, 1.0)
            .expect("space render elements");

    // Shared: count upper layer elements
    let num_upper = count_upper_layer_render_elements(renderer, output);

    // Shared: get element counts for pre-allocation
    let counts = get_render_element_counts(&scene, space_render_elements.len(), num_upper);
    let mut render_elements = Vec::with_capacity(counts.total());

    // Shared: assemble elements in z-order
    super::assemble_scene_elements!(
        WaylandExtras,
        scene,
        space_render_elements,
        num_upper,
        render_elements
    );

    // Backend-specific: render cursor overlays (client surface cursors and DnD icons)
    render_cursor_overlays(
        renderer,
        &cursor_presentation,
        state.pointer.current_location(),
        &mut render_elements,
    );

    // Backend-specific: render with damage tracker
    let render_result = damage_tracker
        .render_output(
            renderer,
            &mut framebuffer,
            buffer_age,
            &render_elements,
            [0.05, 0.05, 0.07, 1.0],
        )
        .expect("render output");

    // Shared: submit pending screencopies
    crate::backend::wayland::compositor::screencopy::submit_pending_screencopies(
        &mut state.pending_screencopies,
        renderer,
        &framebuffer,
        output,
        start_time,
    );

    // Get damage before framebuffer is dropped
    let damage = render_result.damage.cloned();

    // Drop framebuffer before we can use backend again
    drop(framebuffer);

    // Backend-specific: submit buffer
    backend.submit(damage.as_deref()).ok();

    // Shared: send frame callbacks
    send_frame_callbacks(state, output, start_time.elapsed());
}

// Backend-specific: cursor handling via winit window API
fn apply_cursor_presentation_internal(
    backend: &WinitGraphicsBackend<GlesRenderer>,
    presentation: &CursorPresentation,
) {
    match presentation {
        CursorPresentation::Hidden => {
            backend.window().set_cursor_visible(false);
        }
        CursorPresentation::Named(icon) => {
            backend.window().set_cursor_visible(true);
            backend.window().set_cursor(*icon);
        }
        CursorPresentation::Surface { .. } => {
            // Client-provided surface cursor. Winit cannot set surface as cursor,
            // so we hide the system cursor and render as an overlay ourselves in render_frame.
            backend.window().set_cursor_visible(false);
        }
        CursorPresentation::DndIcon { cursor, .. } => {
            // Recursively apply the visibility settings of the base cursor.
            apply_cursor_presentation_internal(backend, cursor);
        }
    }
}

/// Render everything in the cursor presentation that requires manual compositing
/// (client surface cursors and drag-and-drop icons).
fn render_cursor_overlays(
    renderer: &mut GlesRenderer,
    presentation: &CursorPresentation,
    pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    render_elements: &mut Vec<WaylandExtras>,
) {
    match presentation {
        CursorPresentation::Hidden | CursorPresentation::Named(_) => {}
        CursorPresentation::Surface { surface, hotspot } => {
            let cursor_loc = smithay::utils::Point::<i32, smithay::utils::Physical>::from((
                (pointer_location.x - hotspot.x as f64).round() as i32,
                (pointer_location.y - hotspot.y as f64).round() as i32,
            ));
            let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                    renderer,
                    surface,
                    cursor_loc,
                    smithay::utils::Scale::from(1.0),
                    1.0,
                    smithay::backend::renderer::element::Kind::Cursor,
                );
            for elem in elements {
                render_elements.push(WaylandExtras::Surface(elem));
            }
        }
        CursorPresentation::DndIcon {
            icon,
            hotspot,
            cursor,
        } => {
            // Render the base cursor overlay first if it's a surface
            render_cursor_overlays(renderer, cursor, pointer_location, render_elements);

            // Then render the drag icon
            let dnd_loc = smithay::utils::Point::<i32, smithay::utils::Physical>::from((
                (pointer_location.x - hotspot.x as f64).round() as i32,
                (pointer_location.y - hotspot.y as f64).round() as i32,
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
                render_elements.push(WaylandExtras::Surface(elem));
            }
        }
    }
}
