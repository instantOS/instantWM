use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::WinitGraphicsBackend;
use smithay::desktop::space::render_output;
use smithay::output::Output;

use crate::backend::wayland::compositor::WaylandState;
use crate::startup::common_wayland::{
    build_common_scene_elements, resolve_cursor_presentation, send_frame_callbacks,
    CursorPresentation,
};
use crate::wm::Wm;

render_elements! {
    pub WaylandExtras<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
}

pub(super) fn render_frame(
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

        // custom_elements is in front-to-back order: index 0 is the topmost element.
        // The ordering below must be maintained:
        //   1. Overlay windows (dmenu, popups) — above the bar, below the cursor
        //   2. Bar
        //   3. Window borders
        //
        // In the winit backend the cursor is a host-compositor hardware cursor and
        // is always on top by nature, so it does not appear in this list.
        //
        // Overlay windows MUST come before the bar.  The bar is a custom_element
        // that sits above ALL space elements in Smithay's front-to-back list.
        // Without lifting overlays out here, dmenu and similar X11 launchers are
        // drawn behind the bar and appear invisible.
        //
        // DO NOT reorder these sections.
        let scene = build_common_scene_elements(wm, state, renderer, 0);
        let mut custom_elements: Vec<WaylandExtras> = Vec::new();
        for elem in scene.overlays {
            custom_elements.push(WaylandExtras::Surface(elem));
        }
        for elem in scene.bar {
            custom_elements.push(WaylandExtras::Memory(elem));
        }
        for elem in scene.borders {
            custom_elements.push(WaylandExtras::Solid(elem));
        }

        let render_result = render_output(
            output,
            renderer,
            &mut framebuffer,
            1.0,
            buffer_age,
            [&state.space],
            &custom_elements,
            damage_tracker,
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
