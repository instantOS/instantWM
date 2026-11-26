//! Model transitions for the compositor's interaction outline.
use crate::config::config_toml::AnimationConfig;
use crate::core_state::InteractionState;
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
        animate_transition: bool,
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
        let animate = previous.is_some()
            && rect.is_some()
            && animations.enabled
            && animate_transition
            && self.layout_preview_style == style;
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

    #[test]
    fn outline_animation_requires_an_existing_outline_and_both_switches() {
        let target = Rect::new(100, 0, 50, 50);
        for (animate, enabled, existing, expected) in [
            (false, true, true, false),
            (true, false, true, false),
            (true, true, false, false),
            (true, true, true, true),
        ] {
            let mut state = InteractionState {
                layout_preview: existing.then_some(Rect::new(0, 0, 50, 50)),
                ..InteractionState::default()
            };
            let config = AnimationConfig {
                enabled,
                speed: 0.5.try_into().unwrap(),
                ..AnimationConfig::default()
            };
            let projection = state
                .update_outline(
                    &config,
                    animate,
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
                .update_outline(&config, false, Some(rect), InteractionOutlineStyle::Close)
                .is_some()
        );
        assert!(
            state
                .update_outline(&config, false, Some(rect), InteractionOutlineStyle::Close)
                .is_none()
        );
        assert!(
            state
                .update_outline(&config, false, None, InteractionOutlineStyle::Layout)
                .is_some()
        );
        assert!(
            state
                .update_outline(&config, false, None, InteractionOutlineStyle::Close)
                .is_none()
        );
    }
}
