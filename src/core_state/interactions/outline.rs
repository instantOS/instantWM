//! Model transitions for the compositor's interaction outline.
use crate::config::config_toml::AnimationConfig;
use crate::core_state::{ActiveWmMode, InteractionState};
use crate::types::{InteractionOutlineStyle, Rect};
use std::time::Duration;

/// Owned presentation data; no model borrow survives native projection.
#[must_use = "outline transitions must be projected before the next input event"]
pub(crate) struct OutlineProjection {
    pub rect: Option<Rect>,
    pub style: InteractionOutlineStyle,
    pub animate: bool,
    pub duration: Duration,
}

impl InteractionState {
    pub(crate) fn update_outline(
        &mut self,
        animations: &AnimationConfig,
        mode: &ActiveWmMode,
        rect: Option<Rect>,
        style: InteractionOutlineStyle,
    ) -> Option<OutlineProjection> {
        // Clearing the outline invalidates placement even when it was already
        // absent. Keep this before the redundant-presentation check.
        if rect.is_none() {
            self.pointer_placement_cache = None;
        }
        let previous = self.layout_preview;
        if previous == rect && (rect.is_none() || self.layout_preview_style == style) {
            return None;
        }
        // Keyboard placement moves between discrete targets. Pointer-driven
        // previews must follow physical motion without interpolation.
        let animate = previous.is_some()
            && rect.is_some()
            && animations.enabled
            && mode.tree_placement().is_some();
        self.layout_preview = rect;
        self.layout_preview_style = style;
        Some(OutlineProjection {
            rect,
            style,
            animate,
            duration: animations.scale_duration(Duration::from_millis(
                crate::constants::animation::WAYLAND_DEFAULT_ANIMATION_MILLIS,
            )),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_state::KeyboardTreePlacement;
    use crate::layouts::tree::PlacementTarget;
    use crate::types::{MonitorId, Point, TagMask, WindowId};

    fn placement_mode() -> ActiveWmMode {
        ActiveWmMode::TreePlacement(
            KeyboardTreePlacement::new(
                WindowId(1),
                MonitorId::default(),
                TagMask::single(1).unwrap(),
                vec![PlacementTarget {
                    target: WindowId(2),
                    side: None,
                    candidate_index: 0,
                    position: Point::new(0, 0),
                }],
                0,
            )
            .unwrap(),
        )
    }

    #[test]
    fn outline_animation_is_reserved_for_existing_keyboard_targets() {
        let target = Rect::new(100, 0, 50, 50);
        for (mode, enabled, existing, expected) in [
            (ActiveWmMode::Default, true, true, false),
            (ActiveWmMode::Overview, true, true, false),
            (placement_mode(), false, true, false),
            (placement_mode(), true, false, false),
            (placement_mode(), true, true, true),
        ] {
            let mut state = InteractionState {
                layout_preview: existing.then_some(Rect::new(0, 0, 50, 50)),
                ..InteractionState::default()
            };
            let config = AnimationConfig {
                enabled,
                speed: 0.5.try_into().unwrap(),
            };
            let projection = state
                .update_outline(
                    &config,
                    &mode,
                    Some(target),
                    InteractionOutlineStyle::Layout,
                )
                .unwrap();
            assert_eq!(projection.animate, expected);
            assert_eq!(
                projection.duration,
                config.scale_duration(Duration::from_millis(
                    crate::constants::animation::WAYLAND_DEFAULT_ANIMATION_MILLIS,
                ))
            );
            assert_eq!(state.layout_preview, Some(target));
        }
    }

    #[test]
    fn outline_style_changes_are_projected_but_redundant_updates_are_not() {
        let rect = Rect::new(0, 0, 50, 50);
        let mut state = InteractionState {
            layout_preview: Some(rect),
            ..InteractionState::default()
        };
        let config = AnimationConfig::default();
        assert!(
            state
                .update_outline(
                    &config,
                    &ActiveWmMode::Default,
                    Some(rect),
                    InteractionOutlineStyle::Close
                )
                .is_some()
        );
        assert!(
            state
                .update_outline(
                    &config,
                    &ActiveWmMode::Default,
                    Some(rect),
                    InteractionOutlineStyle::Close
                )
                .is_none()
        );
        assert!(
            state
                .update_outline(
                    &config,
                    &ActiveWmMode::Default,
                    None,
                    InteractionOutlineStyle::Layout
                )
                .is_some()
        );
        assert!(
            state
                .update_outline(
                    &config,
                    &ActiveWmMode::Default,
                    None,
                    InteractionOutlineStyle::Close
                )
                .is_none()
        );
    }
}
