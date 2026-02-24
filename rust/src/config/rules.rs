//! Window placement rules.
//!
//! Rules are matched against newly-mapped windows in order.  The first
//! matching rule wins.  A `None` field is a wildcard that matches anything.

use super::commands::SCRATCHPAD_CLASS;
use crate::types::{MonitorRule, Rule, RuleFloat};

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
        // --- Centered floating ---
        Rule {
            class: Some("instantfloat"),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::FloatCenter,
            monitor: MonitorRule::Any,
        },
        // --- Scratchpad ---
        Rule {
            class: Some(SCRATCHPAD_CLASS),
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
        class: Some(class),
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
        class: Some(class),
        instance: None,
        title: None,
        tags: 0,
        isfloating: RuleFloat::FloatFullscreen,
        monitor: MonitorRule::Any,
    }
}
