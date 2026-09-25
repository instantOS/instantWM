//! Per-output tag-bar display policy.
//!
//! Tag display is configured in layers, resolved here once per output:
//!
//! 1. `[bar]` defaults (`show`, `tag_slots`) — all outputs.
//! 2. `[monitors.<name>]` overrides (or `[monitors."*"]` for every output) —
//!    omitted `tag_slots` inherits.
//!
//! Bar visibility is seeded into monitors; `tag_slots` is resolved when the
//! bar renders, exactly like `tags.show_icons`.

use crate::core_state::EffectiveConfig;
use crate::types::Monitor;

/// The effective bar and tag-display settings for one output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagBarPolicy {
    /// Whether the bar is shown on this output at all. The fallback for tag
    /// masks without a session override; not per-output configurable yet.
    pub show_bar: bool,
    /// Number of leading tags considered even if they are empty. Occupied
    /// and selected tags outside this baseline are included too.
    pub tag_slots: u32,
}

impl TagBarPolicy {
    /// Resolve the policy for output `name`, field by field:
    /// `[monitors.<name>]` overrides `[monitors."*"]` overrides the global
    /// `[bar]` defaults. Each layer only supplies the fields it sets, so a
    /// named entry that adjusts one setting keeps inheriting the rest.
    pub fn resolve(config: &EffectiveConfig, name: &str) -> Self {
        let wildcard = config.monitors.get("*");
        let named = config.monitors.get(name);
        Self {
            show_bar: config.bar.show,
            tag_slots: named
                .and_then(|entry| entry.tag_slots)
                .or_else(|| wildcard.and_then(|entry| entry.tag_slots))
                .unwrap_or(config.bar.tag_slots),
        }
    }

    /// Seed the monitor's session state from this policy.
    ///
    /// Per-view `toggle_bar` overrides live in `Monitor::per_tag` and are
    /// cleared by reload and explicit bar configuration writes.
    pub fn apply_to(&self, monitor: &mut Monitor) {
        monitor.bar_default_show = self.show_bar;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::config_toml::{MonitorConfig, UserConfig};
    use crate::config::resolve_config;

    fn config_with(monitors: &[(&str, Option<u32>)]) -> EffectiveConfig {
        let mut user: UserConfig = toml::from_str("").unwrap();
        user.monitors = monitors
            .iter()
            .map(|(name, tag_slots)| {
                (
                    (*name).to_string(),
                    MonitorConfig {
                        tag_slots: *tag_slots,
                        ..MonitorConfig::default()
                    },
                )
            })
            .collect();
        resolve_config(user, crate::backend::BackendKind::Wayland).unwrap()
    }

    #[test]
    fn outputs_inherit_the_global_defaults() {
        let config = config_with(&[]);
        let policy = TagBarPolicy::resolve(&config, "DP-1");
        assert_eq!(policy.tag_slots, crate::types::tag::DEFAULT_TAG_SLOTS);
    }

    #[test]
    fn named_entries_override_and_omitted_fields_inherit() {
        let config = config_with(&[("DP-1", Some(5))]);
        let policy = TagBarPolicy::resolve(&config, "DP-1");
        assert_eq!(policy.tag_slots, 5);

        // The untouched field still inherits the global default.
        let config = config_with(&[("DP-1", None)]);
        let policy = TagBarPolicy::resolve(&config, "DP-1");
        assert_eq!(policy.tag_slots, crate::types::tag::DEFAULT_TAG_SLOTS);
    }

    #[test]
    fn the_wildcard_entry_applies_to_unnamed_outputs() {
        let config = config_with(&[("*", Some(7))]);
        let policy = TagBarPolicy::resolve(&config, "HDMI-1");
        assert_eq!(policy.tag_slots, 7);

        // A named entry still wins over the wildcard.
        let config = config_with(&[("*", Some(7)), ("DP-1", None)]);
        let policy = TagBarPolicy::resolve(&config, "DP-1");
        assert_eq!(policy.tag_slots, 7);
    }

    #[test]
    fn apply_to_seeds_the_configured_bar_visibility() {
        let config = config_with(&[("DP-1", None)]);
        let mut monitor = Monitor::default();

        TagBarPolicy::resolve(&config, "DP-1").apply_to(&mut monitor);
        assert!(monitor.bar_default_show);

        let mut hidden = config.clone();
        hidden.bar.show = false;
        TagBarPolicy::resolve(&hidden, "DP-1").apply_to(&mut monitor);
        assert!(!monitor.bar_default_show);
    }
}
