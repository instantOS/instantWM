//! Window rule types.
//!
//! Types for defining and matching window rules.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// Floating behavior for window rules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleFloat {
    /// Tiled window.
    #[default]
    Tiled,
    /// Floating window.
    Float,
    /// Centered floating window.
    FloatCenter,
    /// Fullscreen floating window.
    FloatFullscreen,
    /// Scratchpad window.
    Scratchpad,
}

/// Monitor selection in rules.
///
/// Replaces the old `i32` field where `-1` meant "any monitor" and `0+` meant
/// a specific monitor index.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorRule {
    /// Place on any available monitor (was -1).
    #[default]
    Any,
    /// Place on specific monitor by index.
    Index(usize),
}

/// A window matching rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Window class to match.
    pub class: Option<Cow<'static, str>>,
    /// Window instance to match.
    pub instance: Option<Cow<'static, str>>,
    /// Window title to match.
    pub title: Option<Cow<'static, str>>,
    /// Tags to assign to matched windows.
    #[serde(default)]
    pub tags: u32,
    /// Floating behavior for matched windows.
    #[serde(default)]
    pub isfloating: RuleFloat,
    /// Monitor placement rule.
    #[serde(default)]
    pub monitor: MonitorRule,
}
