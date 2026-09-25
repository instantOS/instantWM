//! Per-output tag-bar display policy.
//!
//! Tag display is configured in layers, resolved here once per output:
//!
//! 1. `[bar]` defaults (`show_empty_tags`, `tag_slots`) — all outputs.
//! 2. `[monitors.<name>]` overrides (or `[monitors."*"]` for every output) —
//!    omitted fields inherit.
//! 3. The session layer: the `toggle_hide_tags` action flips
//!    [`Monitor::hide_tags`](crate::types::Monitor::hide_tags) on the
//!    selected output until the next policy apply or reload.
//!
//! [`TagBarPolicy::apply_to`] is the only writer seeded from configuration,
//! so a per-output override cannot be forgotten at one of the lifecycle
//! points. `tag_slots` is stateless: the bar resolves it per monitor while
//! rendering, exactly like `tags.show_icons`.

use crate::core_state::EffectiveConfig;
use crate::types::Monitor;

/// The effective tag-display settings for one output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagBarPolicy {
    /// Show tags with no windows that are not selected.
    pub show_empty_tags: bool,
    /// Number of tag cells in this output's bar.
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
            show_empty_tags: named
                .and_then(|entry| entry.show_empty_tags)
                .or_else(|| wildcard.and_then(|entry| entry.show_empty_tags))
                .unwrap_or(config.bar.show_empty_tags),
            tag_slots: named
                .and_then(|entry| entry.tag_slots)
                .or_else(|| wildcard.and_then(|entry| entry.tag_slots))
                .unwrap_or(config.bar.tag_slots),
        }
    }

    /// Seed the monitor's session state from this policy.
    ///
    /// Called on startup, on reload, when an output appears, and whenever
    /// the policy inputs change (`config set bar.*`, `config set
    /// monitors.*`). A `toggle_hide_tags` override applied in between is
    /// intentionally discarded — configuration re-seeds, the session layer
    /// does not persist.
    pub fn apply_to(&self, monitor: &mut Monitor) {
        monitor.hide_tags = !self.show_empty_tags;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::config_toml::{MonitorConfig, UserConfig};
    use crate::config::resolve_config;

    fn config_with(monitors: &[(&str, Option<bool>, Option<u32>)]) -> EffectiveConfig {
        let mut user: UserConfig = toml::from_str("").unwrap();
        user.monitors = monitors
            .iter()
            .map(|(name, show_empty_tags, tag_slots)| {
                (
                    (*name).to_string(),
                    MonitorConfig {
                        show_empty_tags: *show_empty_tags,
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
        assert!(policy.show_empty_tags);
        assert_eq!(
            policy.tag_slots,
            crate::types::tag::DEFAULT_TAG_SLOTS
        );
    }

    #[test]
    fn named_entries_override_and_omitted_fields_inherit() {
        let config = config_with(&[("DP-1", Some(false), Some(5))]);
        let policy = TagBarPolicy::resolve(&config, "DP-1");
        assert!(!policy.show_empty_tags);
        assert_eq!(policy.tag_slots, 5);

        // The untouched field still inherits the global default.
        let config = config_with(&[("DP-1", None, None)]);
        let policy = TagBarPolicy::resolve(&config, "DP-1");
        assert!(policy.show_empty_tags);
    }

    #[test]
    fn the_wildcard_entry_applies_to_unnamed_outputs() {
        let config = config_with(&[("*", Some(false), Some(7))]);
        let policy = TagBarPolicy::resolve(&config, "HDMI-1");
        assert!(!policy.show_empty_tags);
        assert_eq!(policy.tag_slots, 7);

        // A named entry still wins over the wildcard.
        let config = config_with(&[("*", Some(false), Some(7)), ("DP-1", Some(true), None)]);
        let policy = TagBarPolicy::resolve(&config, "DP-1");
        assert!(policy.show_empty_tags);
        assert_eq!(policy.tag_slots, 7);
    }

    #[test]
    fn apply_to_seeds_hide_tags_from_the_policy() {
        let config = config_with(&[("DP-1", Some(false), None)]);
        let mut monitor = Monitor::default();

        TagBarPolicy::resolve(&config, "DP-1").apply_to(&mut monitor);
        assert!(monitor.hide_tags);

        TagBarPolicy::resolve(&config, "HDMI-1").apply_to(&mut monitor);
        assert!(!monitor.hide_tags);
    }
}
