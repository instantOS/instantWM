//! Diagnostic geometry for tiled placement, captured only while enabled.

use std::rc::Rc;
use std::sync::Arc;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::utils::Transform;

use crate::bar::canvas::Canvas;
use crate::core_state::CoreState;
use crate::layouts::tree::{PlacementHitboxes, Side};
use crate::types::{Point, Rect, Rgba, Rgba8};

pub(super) enum PlacementOverlay {
    Pointer {
        hitboxes: Arc<PlacementHitboxes>,
        buffer: MemoryRenderBuffer,
    },
    Keyboard {
        positions: Vec<Point>,
        selected: Point,
        markers: Vec<SolidColorRenderElement>,
    },
}

impl PlacementOverlay {
    pub(super) fn capture(state: &mut CoreState, cached: Option<&Rc<Self>>) -> Option<Rc<Self>> {
        if !state.config.layout.show_placement_hitboxes {
            return None;
        }
        if let Some(placement) = state.behavior.current_mode.tree_placement() {
            let positions = placement.targets().iter().map(|target| target.position);
            let selected = placement.selected_target().position;
            if let Some(cached) = cached
                && let Self::Keyboard {
                    positions: previous,
                    selected: previous_selected,
                    ..
                } = cached.as_ref()
                && selected == *previous_selected
                && positions.clone().eq(previous.iter().copied())
            {
                return Some(Rc::clone(cached));
            }
            let mut markers = Vec::new();
            // The selected anchor is drawn in front of nearby alternatives.
            push_marker(&mut markers, selected, 11, Rgba::rgb(1.0, 0.82, 0.27));
            for point in positions.clone().filter(|point| *point != selected) {
                push_marker(&mut markers, point, 5, Rgba::rgb(0.40, 0.88, 1.0));
            }
            return Some(Rc::new(Self::Keyboard {
                positions: positions.collect(),
                selected,
                markers,
            }));
        }
        let hitboxes = crate::layouts::manager::pointer_placement_hitboxes(state)?;
        if let Some(cached) = cached
            && let Self::Pointer {
                hitboxes: previous, ..
            } = cached.as_ref()
            && Arc::ptr_eq(&hitboxes, previous)
        {
            return Some(Rc::clone(cached));
        }
        let buffer = draw_hitboxes(&hitboxes);
        Some(Rc::new(Self::Pointer { hitboxes, buffer }))
    }
}

fn push_marker(out: &mut Vec<SolidColorRenderElement>, point: Point, width: i32, color: Rgba) {
    let rect = Rect::new(point.x - width / 2, point.y - width / 2, width, width);
    super::borders::push_solid(out, rect, color);
    super::borders::push_solid(
        out,
        Rect::new(rect.x - 1, rect.y - 1, rect.w + 2, rect.h + 2),
        Rgba::rgb(0.08, 0.09, 0.12),
    );
}

fn draw_hitboxes(hitboxes: &PlacementHitboxes) -> MemoryRenderBuffer {
    let mut canvas = Canvas::new(hitboxes.area.size());
    for span in &hitboxes.spans {
        let rect = Rect::new(
            span.rect.x - hitboxes.area.x,
            span.rect.y - hitboxes.area.y,
            span.rect.w,
            span.rect.h,
        );
        let (r, g, b) = match span.side {
            Some(Side::Left) => (91, 231, 193),
            Some(Side::Right) => (108, 173, 255),
            Some(Side::Top) => (255, 190, 110),
            Some(Side::Bottom) => (251, 128, 169),
            None => (198, 169, 255),
        };
        let alpha = if span.band % 2 == 0 { 50 } else { 100 };
        canvas.fill_rect(rect, Rgba8::new(r, g, b, alpha));
        canvas.fill_rect(
            Rect::new(rect.x, rect.y, 1, rect.h),
            Rgba8::new(r, g, b, 165),
        );
    }
    MemoryRenderBuffer::from_slice(
        canvas.as_slice(),
        Fourcc::Argb8888,
        (hitboxes.area.w, hitboxes.area.h),
        1,
        Transform::Normal,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layouts::tree::Preset;
    use crate::test_support::{TestWm, add_client, push_monitor_with};
    use crate::types::{
        Client, ClientMode, InteractionSource, ModMask, MouseButton, TagMask, WindowId,
    };

    #[test]
    fn diagnostics_follow_live_drag_and_keyboard_placement_without_background_work() {
        let mut wm = TestWm::new(crate::backend::WaylandBackendData::default());
        let area = Rect::new(-500, 100, 500, 400);
        let tags = TagMask::single(1).unwrap();
        let monitor = push_monitor_with(&mut wm.core.state.model, |monitor| {
            monitor.monitor_rect = area;
            monitor.available_rect = area;
            monitor.bar_default_show = false;
            monitor.set_selected_tags(tags);
        });
        let windows = [WindowId(1), WindowId(2), WindowId(3)];
        for win in windows.into_iter().rev() {
            add_client(
                &mut wm.core.state.model,
                monitor,
                Client {
                    win,
                    tags,
                    mode: ClientMode::tiled(),
                    ..Client::default()
                },
            );
        }
        let monitor = wm.core.state.model.monitor_mut(monitor).unwrap();
        monitor.selected = Some(windows[0]);
        let tree = &mut monitor.per_tag_state().layout_tree;
        tree.apply_preset(Preset::MasterStack, &windows, 1);
        let source_geo = tree.bounds(area)[&windows[0]];
        wm.core
            .state
            .interaction
            .drag
            .begin_move(
                windows[0],
                MouseButton::Left,
                InteractionSource::Pointer,
                source_geo.center(),
                source_geo,
            )
            .unwrap();

        assert!(PlacementOverlay::capture(&mut wm.core.state, None).is_none());
        assert!(wm.core.state.interaction.pointer_placement_cache.is_none());

        wm.core.state.config.layout.show_placement_hitboxes = true;
        let overlay = PlacementOverlay::capture(&mut wm.core.state, None).unwrap();
        let PlacementOverlay::Pointer { hitboxes, .. } = overlay.as_ref() else {
            panic!("an active tiled move should show pointer hitboxes");
        };
        assert_eq!(hitboxes.area, area);
        assert!(!hitboxes.spans.is_empty());
        assert!(
            hitboxes
                .spans
                .iter()
                .all(|span| !span.rect.contains_point(source_geo.center()))
        );
        let cached = PlacementOverlay::capture(&mut wm.core.state, Some(&overlay)).unwrap();
        assert!(Rc::ptr_eq(&overlay, &cached));
        wm.core.state.config.layout.show_placement_hitboxes = false;
        assert!(PlacementOverlay::capture(&mut wm.core.state, Some(&overlay)).is_none());
        assert!(crate::mouse::drag::active_drag_finish(
            &mut wm.test_ctx(),
            MouseButton::Left,
            ModMask::NONE
        ));
        wm.core.state.config.layout.show_placement_hitboxes = true;
        assert!(PlacementOverlay::capture(&mut wm.core.state, Some(&overlay)).is_none());

        assert!(crate::layouts::begin_tree_placement(&mut wm.test_ctx()));
        let overlay = PlacementOverlay::capture(&mut wm.core.state, None).unwrap();
        assert!(matches!(
            overlay.as_ref(),
            PlacementOverlay::Keyboard { .. }
        ));
        crate::layouts::cycle_keyboard_tree_placement(&mut wm.test_ctx(), false);
        let next = PlacementOverlay::capture(&mut wm.core.state, Some(&overlay)).unwrap();
        assert!(!Rc::ptr_eq(&overlay, &next));
        crate::layouts::finish_keyboard_tree_placement(&mut wm.test_ctx(), false);
        assert!(PlacementOverlay::capture(&mut wm.core.state, Some(&next)).is_none());
    }
}
