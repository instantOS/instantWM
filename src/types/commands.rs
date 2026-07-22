//! Command action types.
//!
//! Types for toggle actions, special next actions, and other command-related enums.

/// Action to perform on a boolean toggle setting.
///
/// Accepted CLI values (via clap):
/// - `toggle` (default): flip the value
/// - `on` | `true` | `1`: set to true
/// - `off` | `false` | `0`: set to false
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    bincode::Decode,
    bincode::Encode,
    serde::Serialize,
    serde::Deserialize,
    clap::ValueEnum,
)]
pub enum ToggleAction {
    /// Toggle the current value (true → false, false → true).
    #[default]
    Toggle,
    /// Set the value to `false`.
    #[value(name = "off", alias = "false", alias = "0")]
    SetFalse,
    /// Set the value to `true`.
    #[value(name = "on", alias = "true", alias = "1")]
    SetTrue,
}

impl ToggleAction {
    /// Apply this action to a boolean value in-place.
    pub fn apply(self, value: &mut bool) {
        match self {
            Self::Toggle => *value = !*value,
            Self::SetFalse => *value = false,
            Self::SetTrue => *value = true,
        }
    }
}

/// Policy for moving keyboard focus with the pointer.
///
/// This deliberately distinguishes physical pointer motion from pointer focus
/// changes caused by rearranging the scene. Keeping that distinction in the
/// shared model prevents X11 crossing events and Wayland focus refreshes from
/// acquiring subtly different semantics.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    bincode::Decode,
    bincode::Encode,
    serde::Serialize,
    serde::Deserialize,
    clap::ValueEnum,
)]
pub enum FocusFollowsMouseMode {
    /// Never move keyboard focus in response to pointer focus.
    Off,
    /// Move focus only in response to physical pointer motion.
    #[default]
    Normal,
    /// Also move focus when a scene change puts a window below the pointer.
    Force,
}

impl FocusFollowsMouseMode {
    pub const fn allows(self, trigger: HoverFocusTrigger) -> bool {
        match (self, trigger) {
            (Self::Off, _) => false,
            (Self::Normal, HoverFocusTrigger::PointerMotion) => true,
            (Self::Normal, HoverFocusTrigger::SceneChange) => false,
            (Self::Force, _) => true,
        }
    }

    pub const fn is_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// What caused the window below the pointer to be reevaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoverFocusTrigger {
    PointerMotion,
    SceneChange,
}

#[cfg(test)]
mod focus_follows_mouse_tests {
    use super::{FocusFollowsMouseMode as Mode, HoverFocusTrigger as Trigger};

    #[test]
    fn modes_distinguish_pointer_motion_from_scene_changes() {
        assert_eq!(Mode::default(), Mode::Normal);
        assert!(!Mode::Off.allows(Trigger::PointerMotion));
        assert!(!Mode::Off.allows(Trigger::SceneChange));
        assert!(Mode::Normal.allows(Trigger::PointerMotion));
        assert!(!Mode::Normal.allows(Trigger::SceneChange));
        assert!(Mode::Force.allows(Trigger::PointerMotion));
        assert!(Mode::Force.allows(Trigger::SceneChange));
    }
}

/// Special next window action state.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    bincode::Decode,
    bincode::Encode,
    serde::Serialize,
    serde::Deserialize,
    clap::ValueEnum,
)]
pub enum SpecialNext {
    /// No special next action.
    #[default]
    None,
    /// Focus next floating window.
    Float,
}

impl From<u32> for SpecialNext {
    fn from(value: u32) -> Self {
        if value == 0 { Self::None } else { Self::Float }
    }
}
