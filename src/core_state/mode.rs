use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveWmMode {
    Default,
    Overview,
    /// Compositor-owned keyboard placement. Keeping the interaction payload
    /// in the mode makes it impossible for modal input and the advertised WM
    /// mode to disagree.
    TreePlacement(KeyboardTreePlacement),
    Named(String),
}

pub const TREE_PLACEMENT_MODE_NAME: &str = "placement";

impl ActiveWmMode {
    pub fn from_name(name: impl Into<String>) -> Self {
        let name = name.into();
        match name.as_str() {
            "" | "default" => Self::Default,
            crate::overview::OVERVIEW_MODE_NAME => Self::Overview,
            _ => Self::Named(name),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Default => "default",
            Self::Overview => crate::overview::OVERVIEW_MODE_NAME,
            Self::TreePlacement(_) => TREE_PLACEMENT_MODE_NAME,
            Self::Named(name) => name,
        }
    }

    pub fn tree_placement(&self) -> Option<&KeyboardTreePlacement> {
        match self {
            Self::TreePlacement(state) => Some(state),
            _ => None,
        }
    }

    pub fn tree_placement_mut(&mut self) -> Option<&mut KeyboardTreePlacement> {
        match self {
            Self::TreePlacement(state) => Some(state),
            _ => None,
        }
    }

    pub fn tree_placement_is_current_for(&self, model: &WmModel) -> bool {
        self.tree_placement()
            .is_some_and(|placement| placement.is_current_for(model))
    }
}

impl From<&str> for ActiveWmMode {
    fn from(name: &str) -> Self {
        Self::from_name(name)
    }
}

impl From<String> for ActiveWmMode {
    fn from(name: String) -> Self {
        Self::from_name(name)
    }
}

#[cfg(test)]
mod active_wm_mode_tests {
    use super::{ActiveWmMode, KeyboardTreePlacement};
    use crate::layouts::tree::{PlacementTarget, Side};
    use crate::types::{MonitorId, Point, TagMask, WindowId};

    #[test]
    fn external_mode_names_are_normalized_into_explicit_states() {
        assert_eq!(ActiveWmMode::from_name(""), ActiveWmMode::Default);
        assert_eq!(ActiveWmMode::from_name("default"), ActiveWmMode::Default);
        assert_eq!(ActiveWmMode::from_name("overview"), ActiveWmMode::Overview);
        assert_eq!(
            ActiveWmMode::from_name("resize"),
            ActiveWmMode::Named("resize".to_string())
        );
    }

    #[test]
    fn keyboard_placement_rejects_an_invalid_selection() {
        let target = PlacementTarget {
            target: WindowId(2),
            side: Some(Side::Left),
            candidate_index: 0,
            position: Point::new(10, 20),
        };
        assert!(
            KeyboardTreePlacement::new(
                WindowId(1),
                MonitorId::default(),
                TagMask::EMPTY,
                vec![target],
                1,
            )
            .is_none()
        );

        let mut placement = KeyboardTreePlacement::new(
            WindowId(1),
            MonitorId::default(),
            TagMask::EMPTY,
            vec![target],
            0,
        )
        .expect("valid selection");
        assert_eq!(placement.selected_target(), target);
        assert!(!placement.select(1));
        assert_eq!(placement.selected_target(), target);
    }

    fn target(window: u32, x: i32, y: i32) -> PlacementTarget {
        PlacementTarget {
            target: WindowId(window),
            side: None,
            candidate_index: 0,
            position: Point::new(x, y),
        }
    }

    #[test]
    fn keyboard_placement_nearest_target_is_stable_for_equal_distances() {
        let targets = [target(1, -10, 0), target(2, 10, 0)];
        let placement = KeyboardTreePlacement::new_nearest(
            WindowId(9),
            MonitorId::default(),
            TagMask::EMPTY,
            targets.to_vec(),
            Point::new(0, 0),
        )
        .unwrap();
        assert_eq!(placement.selected_target(), targets[0]);
    }

    #[test]
    fn keyboard_placement_direction_wraps_at_a_visual_edge() {
        let targets = vec![target(1, 0, 0), target(2, 100, 0), target(3, -100, 0)];
        let mut placement = KeyboardTreePlacement::new(
            WindowId(9),
            MonitorId::default(),
            TagMask::EMPTY,
            targets,
            0,
        )
        .unwrap();

        assert!(placement.select_direction(Side::Right));
        assert_eq!(placement.selected_target().target, WindowId(2));
        assert!(placement.select_direction(Side::Top));
        assert_eq!(placement.selected_target().target, WindowId(1));
    }

    #[test]
    fn keyboard_placement_direction_prefers_visual_alignment() {
        let targets = vec![target(1, 0, 0), target(2, 80, 100), target(3, 100, 5)];
        let mut placement = KeyboardTreePlacement::new(
            WindowId(9),
            MonitorId::default(),
            TagMask::EMPTY,
            targets,
            0,
        )
        .unwrap();

        assert!(placement.select_direction(Side::Right));
        assert_eq!(placement.selected_target().target, WindowId(3));
    }

    #[test]
    fn keyboard_placement_wrap_prefers_alignment_on_the_opposite_edge() {
        let targets = vec![
            target(1, 100, 100),
            target(2, 0, 10),
            target(3, 20, 102),
            target(4, 0, 300),
        ];
        let mut placement = KeyboardTreePlacement::new(
            WindowId(9),
            MonitorId::default(),
            TagMask::EMPTY,
            targets,
            0,
        )
        .unwrap();

        assert!(placement.select_direction(Side::Right));
        assert_eq!(placement.selected_target().target, WindowId(2));
    }
}
