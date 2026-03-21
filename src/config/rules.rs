//! Window placement rules.
//!
//! Rules are matched against newly-mapped windows in order.  The first
//! matching rule wins.  A `None` field is a wildcard that matches anything.

use super::commands::SCRATCHPAD_CLASS;
use crate::types::{MonitorRule, Rule, RuleFloat};

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
        float("Pavucontrol"),
        float("Onboard"),
        float("floatmenu"),
        float("Welcome.py"),
        float("Pamac-installer"),
        float("xpad"),
        float("Guake"),
        float("wl-copy"),
        // --- Centered floating ---
        Rule {
            class: Some(Cow::Borrowed("instantfloat")),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::FloatCenter,
            monitor: MonitorRule::Any,
        },
        // --- Scratchpad ---
        Rule {
            class: Some(Cow::Borrowed(SCRATCHPAD_CLASS)),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::Scratchpad,
            monitor: MonitorRule::Any,
        },
        // --- Fullscreen floating (takes full screen but stays floating) ---
        fullscreen_float("kdeconnect.daemon"),
        fullscreen_float("Panther"),
        // --- Misc floating ---
        float("org-wellkord-globonote-Main"),
        float("Peek"),
    ]
}

// ---------------------------------------------------------------------------
// Helpers — avoids repeating the full Rule literal for the common cases
// ---------------------------------------------------------------------------

/// A rule that makes `class` float freely.
fn float(class: &'static str) -> Rule {
    Rule {
        class: Some(Cow::Borrowed(class)),
        instance: None,
        title: None,
        tags: 0,
        isfloating: RuleFloat::Float,
        monitor: MonitorRule::Any,
    }
}

/// A rule that makes `class` float at fullscreen size.
fn fullscreen_float(class: &'static str) -> Rule {
    Rule {
        class: Some(Cow::Borrowed(class)),
        instance: None,
        title: None,
        tags: 0,
        isfloating: RuleFloat::FloatFullscreen,
        monitor: MonitorRule::Any,
    }
}
