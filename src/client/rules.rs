//! Window rule application and matching logic.

use crate::globals::Globals;
use crate::types::{MonitorRule, Rect, RuleFloat, SpecialNext, TagMask, WindowId};

/// Properties used for rule matching.
#[derive(Debug, Clone, Default)]
pub struct WindowProperties {
    pub class: String,
    pub instance: String,
    pub title: String,
}

/// Apply the configured window rules to `win`.
///
/// Rules are matched against the window's properties (class, instance, title).
/// Matching rules can set:
///
/// * `isfloating` / layout override (`RuleFloat` variant).
/// * Tag mask (`tags` field).
/// * Target monitor (`monitor` field).
///
/// After rule matching, the final tag mask is clamped to the current tag set.
/// If no rule matches (and `SpecialNext` is `None`), the window inherits its
/// monitor's currently active tags.
pub fn apply_rules(g: &mut Globals, win: WindowId, props: &WindowProperties) {
    // --- Initialise fields we are about to set -------------------------------
    if let Some(c) = g.clients.get_mut(&win) {
        if !props.title.is_empty() {
            c.name = props.title.clone();
        }

        // Scratchpad state is a runtime role assigned after manage. On Wayland
        // we may see later title/app_id updates that re-run this function; do
        // not let those rule refreshes retag an existing scratchpad back into
        // a normal window.
        if c.has_scratchpad_identity() {
            return;
        }

        c.is_floating = false;
        c.set_tag_mask(crate::types::TagMask::EMPTY);
    }

    let special_next = g.behavior.specialnext;
    let rules = g.cfg.rules.clone();
    let tag_mask = TagMask::from_bits(g.tags.mask());
    let bar_height = g.cfg.bar_height;

    // --- Handle SpecialNext shortcut or normal rule matching -----------------
    if special_next != SpecialNext::None {
        if let SpecialNext::Float = special_next
            && let Some(c) = g.clients.get_mut(&win)
        {
            c.is_floating = true;
        }
        g.behavior.specialnext = SpecialNext::None;
    } else {
        for rule in &rules {
            if !rule_matches(rule, props) {
                continue;
            }

            // Special case: Onboard (on-screen keyboard) is always sticky.
            if rule.class.as_deref() == Some("Onboard")
                && let Some(c) = g.clients.get_mut(&win)
            {
                c.issticky = true;
            }

            // Look up monitor geometry for FloatFullscreen / Float rules.
            let mon_geo = g
                .clients
                .monitor_id(win)
                .and_then(|mid| g.monitor(mid))
                .map(|m| (m.monitor_rect, m.work_rect, m.showbar));

            if let Some(c) = g.clients.get_mut(&win) {
                apply_float_rule(c, &rule.isfloating, mon_geo, bar_height);
                c.update_tag_mask(|tags| tags | rule.tags);
            }

            apply_monitor_rule(g, win, rule);
        }
    }

    // --- Clamp tags to the valid tag mask ------------------------------------
    clamp_client_tags(g, win, tag_mask);
}

/// Refresh rule-derived metadata after a backend property update.
///
/// Backend callbacks such as Wayland `title_changed` / `app_id_changed`
/// should use this instead of blindly re-running full window classification.
/// Once a client has been promoted to a scratchpad, later protocol metadata
/// churn must not retag it back into a normal window.
pub fn refresh_rules_for_property_change(g: &mut Globals, win: WindowId, props: &WindowProperties) {
    if let Some(c) = g.clients.get_mut(&win)
        && !props.title.is_empty()
    {
        c.name = props.title.clone();
    }

    if g.clients
        .get(&win)
        .is_some_and(|c| c.has_scratchpad_identity())
    {
        return;
    }

    apply_rules(g, win, props);
}

/// Return `true` when `rule` matches all provided window identifiers.
///
/// Each criterion is optional; an absent criterion always matches.
fn rule_matches(rule: &crate::types::Rule, props: &WindowProperties) -> bool {
    let title_match = rule
        .title
        .as_ref()
        .map(|t| bytes_contains(props.title.as_bytes(), t))
        .unwrap_or(true);
    let class_match = rule
        .class
        .as_ref()
        .map(|c| bytes_contains(props.class.as_bytes(), c))
        .unwrap_or(true);
    let instance_match = rule
        .instance
        .as_ref()
        .map(|i| bytes_contains(props.instance.as_bytes(), i))
        .unwrap_or(true);

    title_match && class_match && instance_match
}

/// Return `true` when `needle` appears as a contiguous subsequence of `haystack`.
#[inline]
fn bytes_contains(haystack: &[u8], needle: &str) -> bool {
    let nb = needle.as_bytes();
    haystack.windows(nb.len()).any(|w| w == nb)
}

/// Apply a `RuleFloat` variant to `client`, optionally adjusting its geometry
/// using the monitor information supplied via `mon_geo`.
///
/// `mon_geo` is `(monitor_rect, work_rect, showbar)` and may be `None` when the
/// client is not yet placed on any monitor (geometry adjustments are skipped).
fn apply_float_rule(
    client: &mut crate::types::client::Client,
    float_rule: &RuleFloat,
    mon_geo: Option<(Rect, Rect, bool)>,
    bar_height: i32,
) {
    let (monitor_rect, work_rect, showbar) = mon_geo.unwrap_or_default();

    match float_rule {
        RuleFloat::FloatCenter => {
            client.is_floating = true;
        }
        RuleFloat::FloatFullscreen => {
            client.is_floating = true;
            client.geo.w = monitor_rect.w;
            client.geo.h = work_rect.h;
            client.geo.x = monitor_rect.x;
            if showbar {
                client.geo.y = monitor_rect.y + bar_height;
            }
        }
        RuleFloat::Scratchpad => {
            client.is_floating = true;
        }
        RuleFloat::Float => {
            client.is_floating = true;
            if showbar {
                client.geo.y = monitor_rect.y + bar_height;
            }
        }
        RuleFloat::Tiled => {
            client.is_floating = false;
        }
    }
}

/// Move `win` to the monitor named in `rule.monitor`, if any.
fn apply_monitor_rule(g: &mut Globals, win: WindowId, rule: &crate::types::Rule) {
    let MonitorRule::Index(target_num) = rule.monitor else {
        return;
    };

    let target_mid = g
        .monitors_iter()
        .find(|(_i, m)| m.num == target_num as i32)
        .map(|(i, _)| i);

    if let Some(mid) = target_mid
        && let Some(c) = g.clients.get_mut(&win)
    {
        c.monitor_id = mid;
    }
}

/// Clamp `win`'s tag mask to valid bits and fall back to the monitor's active
/// tags when no rule-assigned tag is currently visible.
fn clamp_client_tags(g: &mut Globals, win: WindowId, tag_mask: TagMask) {
    let (client_mon_id, client_tags) = g
        .clients
        .get(&win)
        .map(|c| (c.monitor_id, TagMask::from_bits(c.tags)))
        .unwrap_or((0, TagMask::EMPTY));

    let Some(mon) = g.monitor(client_mon_id) else {
        return;
    };

    let mut final_tags = client_tags & tag_mask;
    if final_tags.is_empty() {
        final_tags = mon.selected_tags();
    }

    if let Some(c) = g.clients.get_mut(&win) {
        c.set_tag_mask(final_tags);
    }
}
