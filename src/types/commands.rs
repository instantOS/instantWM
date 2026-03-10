//! Command action types.
//!
//! Types for toggle actions, special next actions, and other command-related enums.

/// Action to perform on a boolean toggle setting.
///
/// Replaces the old C pattern where `arg: u32` encoded toggle behavior:
/// - 0 or 2: toggle the value
/// - 1: set to false
/// - else: set to true
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToggleAction {
    /// Toggle the current value (true → false, false → true).
    #[default]
    Toggle,
    /// Set the value to `false`.
    SetFalse,
    /// Set the value to `true`.
    SetTrue,
}

impl ToggleAction {
    /// Parse from a raw u32 value (for compatibility with external commands).
    ///
    /// Returns Toggle for invalid values.
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 | 2 => Self::Toggle,
            1 => Self::SetFalse,
            _ => Self::SetTrue,
        }
    }

    /// Parse from command argument string.
    ///
    /// Empty string defaults to Toggle, otherwise parses as u32.
    pub fn from_arg(arg: &str) -> Self {
        if arg.is_empty() {
            Self::Toggle
        } else {
            arg.parse().ok().map(Self::from_u32).unwrap_or_default()
        }
    }

    /// Apply this action to a boolean value.
    pub fn apply(self, value: &mut bool) {
        match self {
            Self::Toggle => *value = !*value,
            Self::SetFalse => *value = false,
            Self::SetTrue => *value = true,
        }
    }
}

/// Special next window action state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpecialNext {
    /// No special next action.
    #[default]
    None,
    /// Focus next floating window.
    Float,
}
