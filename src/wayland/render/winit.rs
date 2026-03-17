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
    apply_cursor_image_status(backend, state);
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
fn apply_cursor_image_status(backend: &WinitGraphicsBackend<GlesRenderer>, state: &WaylandState) {
    match resolve_cursor_presentation(&state.cursor_image_status, state.cursor_icon_override) {
        CursorPresentation::Hidden => {
            backend.window().set_cursor_visible(false);
        }
        CursorPresentation::Named(icon) => {
            backend.window().set_cursor_visible(true);
            backend.window().set_cursor(icon);
        }
        CursorPresentation::Surface { .. } => {
            backend.window().set_cursor_visible(true);
        }
    }
}
