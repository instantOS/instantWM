//! Window rule types.
//!
//! Types for defining and matching window rules.

/// Floating behavior for window rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleFloat {
    /// Tiled window.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorRule {
    /// Place on any available monitor (was -1).
    Any,
    /// Place on specific monitor by index.
    Index(usize),
}

/// A window matching rule.
#[derive(Debug, Clone)]
pub struct Rule {
    /// Window class to match.
    pub class: Option<&'static str>,
    /// Window instance to match.
    pub instance: Option<&'static str>,
    /// Window title to match.
    pub title: Option<&'static str>,
    /// Tags to assign to matched windows.
    pub tags: u32,
    /// Floating behavior for matched windows.
    pub isfloating: RuleFloat,
    /// Monitor placement rule.
    pub monitor: MonitorRule,
}
