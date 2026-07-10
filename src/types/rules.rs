//! Window rule types.
//!
//! Types for defining and matching window rules.

use crate::types::TagMask;
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
    pub tags: TagMask,
    /// Floating behavior for matched windows.
    #[serde(default)]
    pub isfloating: Option<RuleFloat>,
    /// Monitor placement rule.
    #[serde(default)]
    pub monitor: MonitorRule,
}

impl Rule {
    /// Check if this rule matches the window identifiers.
    pub fn matches(&self, class: &str, instance: &str, title: &str) -> bool {
        let title_match = self
            .title
            .as_ref()
            .map(|t| bytes_contains(title.as_bytes(), t))
            .unwrap_or(true);
        let class_match = self
            .class
            .as_ref()
            .map(|c| bytes_contains(class.as_bytes(), c))
            .unwrap_or(true);
        let instance_match = self
            .instance
            .as_ref()
            .map(|i| bytes_contains(instance.as_bytes(), i))
            .unwrap_or(true);

        title_match && class_match && instance_match
    }
}

#[inline]
fn bytes_contains(haystack: &[u8], needle: &str) -> bool {
    let nb = needle.as_bytes();
    haystack.windows(nb.len()).any(|w| w == nb)
}
