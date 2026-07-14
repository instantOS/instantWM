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
