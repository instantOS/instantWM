//! Window rule types.
//!
//! Types for defining and matching window rules.

use crate::types::TagMask;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::time::Instant;

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
    pub is_floating: Option<RuleFloat>,
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
    // `windows(0)` panics; an empty matcher is degenerate, so treat it as
    // "matches everything" like `str::contains("")`.
    if nb.is_empty() {
        return true;
    }
    haystack.windows(nb.len()).any(|w| w == nb)
}

/// A rule queued at runtime to apply to the next matching window.
///
/// Distinct from config-loaded [`Rule`]s: every entry here has an absolute
/// deadline and is consumed on first match (and only on initial rule
/// application, never on property refresh). Expired entries are dropped
/// lazily by [`crate::client::rules`].
#[derive(Debug, Clone)]
pub struct PendingTmpRule {
    /// Unique id within the WM session, used by `--cancel` and the `--list`
    /// output.
    pub id: u64,
    /// The rule that will be applied on match.
    pub rule: Rule,
    /// Absolute deadline; the entry is dropped when `Instant::now()` passes
    /// this. The CLI always requires a positive TTL.
    pub deadline: Instant,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_matcher_does_not_panic() {
        // `windows(0)` would panic, so an empty matcher must be handled.
        let rule = Rule {
            class: Some(Cow::Borrowed("")),
            instance: None,
            title: None,
            tags: TagMask::EMPTY,
            is_floating: None,
            monitor: MonitorRule::Any,
        };
        // Empty matcher matches everything rather than panicking.
        assert!(rule.matches("anything", "anything", "anything"));
    }
}
