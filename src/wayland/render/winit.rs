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
    build_common_scene_elements, resolve_cursor_presentation, send_frame_callbacks,
    CursorPresentation,
};
use crate::wm::Wm;

render_elements! {
    pub WaylandExtras<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Space=smithay::desktop::space::SpaceRenderElements<GlesRenderer, smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<GlesRenderer>>,
}

pub fn render_frame(
    wm: &mut Wm,
    state: &mut WaylandState,
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
    start_time: std::time::Instant,
) {
    apply_cursor_image_status(backend, state);
    state.tick_window_animations();
    let damage = {
        let buffer_age = backend.buffer_age().unwrap_or(0);
        let (renderer, mut framebuffer) = backend.bind().expect("renderer bind");

        let scene = build_common_scene_elements(wm, state, renderer, 0);

        let space_render_elements =
            smithay::desktop::space::space_render_elements(renderer, [&state.space], output, 1.0)
                .expect("space render elements");

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
                let elems: Vec<
                    smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<
                        GlesRenderer,
                    >,
                > = smithay::backend::renderer::element::AsRenderElements::render_elements(
                    surface,
                    renderer,
                    geo.loc.to_physical_precise_round(output_scale),
                    smithay::utils::Scale::from(output_scale),
                    1.0,
                );
                num_upper += elems.len();
            }
        }

        let mut render_elements = Vec::with_capacity(
            scene.overlays.len()
                + scene.bar.len()
                + scene.borders.len()
                + space_render_elements.len(),
        );

        // 1. Custom overlays (dmenu, popups)
        for elem in scene.overlays {
            render_elements.push(WaylandExtras::Surface(elem));
        }

        // 2. Upper layer shells (Overlay / Top)
        let mut space_iter = space_render_elements.into_iter();
        for elem in space_iter.by_ref().take(num_upper) {
            render_elements.push(WaylandExtras::Space(elem));
        }

        // 3. Status Bar
        for elem in scene.bar {
            render_elements.push(WaylandExtras::Memory(elem));
        }

        // 4. Borders
        for elem in scene.borders {
            render_elements.push(WaylandExtras::Solid(elem));
        }

        // 5. Windows and lower layer shells (Bottom / Background)
        for elem in space_iter {
            render_elements.push(WaylandExtras::Space(elem));
        }

        let render_result = damage_tracker
            .render_output(
                renderer,
                &mut framebuffer,
                buffer_age,
                &render_elements,
                [0.05, 0.05, 0.07, 1.0],
            )
            .expect("render output");

        // Fulfil pending screencopy requests while framebuffer is still bound.
        crate::backend::wayland::compositor::screencopy::submit_pending_screencopies(
            &mut state.pending_screencopies,
            renderer,
            &framebuffer,
            output,
            start_time,
        );

        render_result.damage.cloned()
    };
    let _ = backend.submit(damage.as_deref());

    send_frame_callbacks(state, output, start_time.elapsed());
}

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
