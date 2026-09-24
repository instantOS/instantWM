//! Window placement rules.
//!
//! Rules are matched against newly-mapped windows in order.  The first
//! matching rule wins.  A `None` field is a wildcard that matches anything.

use super::commands::SCRATCHPAD_CLASS;
use crate::types::{MonitorSelector, Rule, RuleFloat, TagMask};

use std::borrow::Cow;

/// Merge default rules with TOML-configured rules.
///
/// TOML rules are prepended to the defaults, so they match first.
pub fn merge_rules(defaults: Vec<Rule>, toml_rules: Vec<Rule>) -> Vec<Rule> {
    let mut rules = toml_rules;
    rules.extend(defaults);
    rules
}

/// Build the list of window placement rules.
pub fn get_rules() -> Vec<Rule> {
    vec![
        // --- Floating dialogs / tools ---
        rule("Pavucontrol", RuleFloat::Float),
        rule("Onboard", RuleFloat::Float),
        rule("floatmenu", RuleFloat::Float),
        rule("Welcome.py", RuleFloat::Float),
        rule("Pamac-installer", RuleFloat::Float),
        rule("xpad", RuleFloat::Float),
        rule("Guake", RuleFloat::Float),
        rule("wl-copy", RuleFloat::Float),
        // --- Centered floating ---
        rule("instantfloat", RuleFloat::FloatCenter),
        // --- Scratchpad ---
        rule(SCRATCHPAD_CLASS, RuleFloat::Scratchpad),
        // --- Fullscreen floating (takes full screen but stays floating) ---
        rule("kdeconnect.daemon", RuleFloat::FloatFullscreen),
        rule("Panther", RuleFloat::FloatFullscreen),
        // --- Misc floating ---
        rule("org-wellkord-globonote-Main", RuleFloat::Float),
        rule("Peek", RuleFloat::Float),
    ]
}

// ---------------------------------------------------------------------------
// Helpers — avoids repeating the full Rule literal for the common cases
// ---------------------------------------------------------------------------

/// A rule that matches `class` and applies `float` as its placement behavior.
fn rule(class: &'static str, float: RuleFloat) -> Rule {
    Rule {
        class: Some(Cow::Borrowed(class)),
        instance: None,
        title: None,
        tags: TagMask::EMPTY,
        is_floating: Some(float),
        monitor: MonitorSelector::Any,
        geometry: None,
        borderless: false,
    }
}
