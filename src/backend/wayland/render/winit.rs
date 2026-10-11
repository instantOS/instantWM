use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::WinitGraphicsBackend;
use smithay::output::Output;

use crate::backend::wayland::compositor::WaylandNativeState;
use crate::backend::wayland::render::cursor::{ResolvedCursor, resolve_cursor};
use crate::backend::wayland::render::frame::{send_frame_callbacks, update_primary_scanout_output};
use crate::backend::wayland::render::scene::{
    SceneCache, build_common_scene_elements, window_render_elements,
};
use crate::wm::WaylandWm as Wm;

render_elements! {
    pub WaylandExtras<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
}

/// Render a frame using the winit backend.
pub fn render_frame(
    wm: &mut Wm,
    state: &mut WaylandNativeState,
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
    scene_cache: &mut SceneCache,
    start_time: std::time::Instant,
) -> bool {
    // Backend-specific: apply cursor via window API
    let resolved_cursor = resolve_cursor(
        &state.cursor_image_status,
        state.cursor_icon_override,
        state.runtime.dnd_icon.as_ref(),
        state.runtime.cursor_hidden_by_touch,
    );
    apply_resolved_cursor_internal(backend, &resolved_cursor);
    // Backend-specific: get buffer age
    let buffer_age = backend.buffer_age().unwrap_or(0);

    // Backend-specific: bind to get framebuffer
    let (renderer, mut framebuffer) = backend.bind().expect("renderer bind");

    let mut render_elements: Vec<WaylandExtras>;

    if state.is_locked() {
        // When locked, only render the lock surface for this output.
        render_elements = Vec::with_capacity(4);
        let output_name = output.name();
        if let Some(lock_surface) = state.lock_surfaces.get(&output_name) {
            let lock_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                    renderer,
                    lock_surface.wl_surface(),
                    smithay::utils::Point::<i32, smithay::utils::Physical>::from((0, 0)),
                    smithay::utils::Scale::from(1.0),
                    1.0,
                    smithay::backend::renderer::element::Kind::Unspecified,
                );
            for elem in lock_elements {
                render_elements.push(WaylandExtras::Surface(elem));
            }
        }
    } else {
        // Shared: build scene elements
        let scene = build_common_scene_elements(wm, state, scene_cache, renderer, output);

        let windows = window_render_elements(state, renderer, output, &scene);
        render_elements = Vec::with_capacity(scene.len() + windows.len() + 2);

        // Backend-specific: render cursor overlays (client surface cursors and DnD icons)
        render_cursor_overlays(
            renderer,
            &resolved_cursor,
            state.pointer.current_location(),
            &mut render_elements,
        );

        // Shared: assemble elements in z-order
        super::assemble_scene_elements!(WaylandExtras, scene, windows, render_elements);
    }

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

    update_primary_scanout_output(state, output, &render_result.states);

    // Shared: submit pending screencopies
    crate::backend::wayland::compositor::screencopy::submit_pending_screencopies(
        &mut state.runtime.pending_screencopies,
        renderer,
        &framebuffer,
        output,
        true,
    );
    crate::backend::wayland::compositor::image_capture::submit_pending_image_captures(
        &mut state.runtime.pending_image_captures,
        renderer,
        &framebuffer,
        output,
        true,
    );
    // Get damage before framebuffer is dropped
    let damage = render_result.damage.cloned();

    // Drop framebuffer before we can use backend again
    drop(framebuffer);

    // `submit(None)` means an unrestricted swap, not "skip this frame".
    // Smithay's damage tracker returns `None` when the scene is unchanged, so
    // only swap when it reports actual damage.
    let submitted = if let Some(damage) = damage.as_deref() {
        match backend.submit(Some(damage)) {
            Ok(()) => true,
            Err(err) => {
                log::warn!("winit frame submission failed: {err}");
                false
            }
        }
    } else {
        false
    };

    if submitted {
        // Shared: frame callbacks follow a buffer submitted for presentation.
        crate::backend::wayland::render::frame::release_fifo_barriers(state, output);
        send_frame_callbacks(state, output, start_time.elapsed());
    }
    submitted
}

// Backend-specific: cursor handling via winit window API
fn apply_resolved_cursor_internal(
    backend: &WinitGraphicsBackend<GlesRenderer>,
    presentation: &ResolvedCursor,
) {
    match presentation {
        ResolvedCursor::Hidden => {
            backend.window().set_cursor_visible(false);
        }
        ResolvedCursor::Named(icon) => {
            backend.window().set_cursor_visible(true);
            backend
                .window()
                .set_cursor(smithay::reexports::winit::cursor::Cursor::Icon(*icon));
        }
        ResolvedCursor::Surface { .. } => {
            // Client-provided surface cursor. Winit cannot set surface as cursor,
            // so we hide the system cursor and render as an overlay ourselves in render_frame.
            backend.window().set_cursor_visible(false);
        }
        ResolvedCursor::DndIcon { cursor, .. } => {
            // Recursively apply the visibility settings of the base cursor.
            apply_resolved_cursor_internal(backend, cursor);
        }
    }
}

/// Render everything in the cursor presentation that requires manual compositing
/// (client surface cursors and drag-and-drop icons).
fn render_cursor_overlays(
    renderer: &mut GlesRenderer,
    presentation: &ResolvedCursor,
    pointer_location: smithay::utils::Point<f64, smithay::utils::Logical>,
    render_elements: &mut Vec<WaylandExtras>,
) {
    match presentation {
        ResolvedCursor::Hidden | ResolvedCursor::Named(_) => {}
        ResolvedCursor::Surface { surface, hotspot } => {
            // Double-check that the surface is still alive before rendering.
            let Some(elements) = super::cursor_surface_render_elements(
                renderer,
                surface,
                pointer_location,
                *hotspot,
                1.0,
            ) else {
                return;
            };
            render_elements.extend(elements.into_iter().map(WaylandExtras::Surface));
        }
        ResolvedCursor::DndIcon {
            icon,
            hotspot,
            cursor,
        } => {
            // Render the base cursor overlay first if it's a surface
            render_cursor_overlays(renderer, cursor, pointer_location, render_elements);

            // Double-check that the drag icon surface is still alive before rendering.
            let Some(dnd_elements) = super::cursor_surface_render_elements(
                renderer,
                icon,
                pointer_location,
                *hotspot,
                1.0,
            ) else {
                return;
            };

            // Then render the drag icon
            render_elements.extend(dnd_elements.into_iter().map(WaylandExtras::Surface));
        }
    }
}
