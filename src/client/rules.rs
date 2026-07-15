//! Window rule application and matching logic.

use crate::client::LaunchContext;
use crate::core_state::CoreState;
use crate::types::{ClientMode, MonitorRule, Rect, RuleFloat, SpecialNext, TagMask, WindowId};

/// Properties used for rule matching.
#[derive(Debug, Clone, Default)]
pub struct WindowProperties {
    pub class: String,
    pub instance: String,
    pub title: String,
}

/// Positioning instruction produced by an initial window rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InitialRulePlacement {
    /// Keep the backend-derived placement policy.
    #[default]
    Default,
    /// Center the new floating window even if X11 supplied a position.
    Center,
    /// Preserve geometry explicitly assigned by the rule.
    Preserve,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InitialRuleOutcome {
    pub changed: bool,
    pub placement: InitialRulePlacement,
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
pub fn apply_rules(
    g: &mut CoreState,
    win: WindowId,
    props: &WindowProperties,
    launch_context: Option<LaunchContext>,
) -> bool {
    apply_rules_impl(g, win, props, launch_context).changed
}

/// Apply rules while retaining the spawn-position instruction needed during
/// initial window management.
pub fn apply_initial_rules(
    g: &mut CoreState,
    win: WindowId,
    props: &WindowProperties,
    launch_context: Option<LaunchContext>,
) -> InitialRuleOutcome {
    apply_rules_impl(g, win, props, launch_context)
}

fn apply_rules_impl(
    g: &mut CoreState,
    win: WindowId,
    props: &WindowProperties,
    launch_context: Option<LaunchContext>,
) -> InitialRuleOutcome {
    let before = rule_state_snapshot(g, win);
    let mut placement = InitialRulePlacement::Default;

    // --- Initialise fields we are about to set -------------------------------
    if let Some(c) = g.model.client_mut(win) {
        if !props.title.is_empty() {
            c.name = props.title.clone();
        }

        // Scratchpad state is a runtime role assigned after manage. On Wayland
        // we may see later title/app_id updates that re-run this function; do
        // not let those rule refreshes retag an existing scratchpad back into
        // a normal window.
        if c.scratchpad.is_some() {
            return InitialRuleOutcome::default();
        }

        c.mode = if launch_context.map(|ctx| ctx.is_floating).unwrap_or(false) {
            ClientMode::Floating
        } else {
            ClientMode::Tiling
        };
        c.set_tag_mask(crate::types::TagMask::EMPTY);
    }

    let special_next = g.behavior.specialnext;
    let rules = g.config.bindings.rules.clone();
    let tag_mask = g.model.tags.mask();
    let bar_height = g.config.derived.bar_height;

    // --- Handle SpecialNext shortcut or normal rule matching -----------------
    if special_next != SpecialNext::None {
        if let SpecialNext::Float = special_next
            && let Some(c) = g.model.client_mut(win)
        {
            c.mode = ClientMode::Floating;
        }
        g.behavior.specialnext = SpecialNext::None;
    } else {
        for rule in &rules {
            if !rule.matches(&props.class, &props.instance, &props.title) {
                continue;
            }

            // Special case: Onboard (on-screen keyboard) is always sticky.
            if rule.class.as_deref() == Some("Onboard")
                && let Some(c) = g.model.client_mut(win)
            {
                c.is_sticky = true;
            }

            // Look up monitor geometry for FloatFullscreen / Float rules.
            let mon_geo = {
                let view = match g.model.client_view(win) {
                    Some(view) => view,
                    None => continue,
                };
                let mon = view.monitor;
                let mask = view.client.tags;
                (mon.monitor_rect, mon.work_rect, mon.show_bar_for_mask(mask))
            };

            if let Some(c) = g.model.client_mut(win) {
                if let Some(ref float_rule) = rule.is_floating {
                    apply_float_rule(c, float_rule, mon_geo, bar_height);
                    placement = match float_rule {
                        RuleFloat::FloatCenter => InitialRulePlacement::Center,
                        RuleFloat::FloatFullscreen => InitialRulePlacement::Preserve,
                        _ => InitialRulePlacement::Default,
                    };
                }
                c.update_tag_mask(|tags| tags | rule.tags);
            }

            apply_monitor_rule(g, win, rule);
            break;
        }
    }

    // --- Clamp tags to the valid tag mask ------------------------------------
    clamp_client_tags(g, win, tag_mask, launch_context);

    InitialRuleOutcome {
        changed: before != rule_state_snapshot(g, win),
        placement,
    }
}

/// Refresh rule-derived metadata after a backend property update.
///
/// Backend callbacks such as Wayland `title_changed` / `app_id_changed`
/// and X11 `PropertyNotify` handlers should route through this shared WM
/// entry point instead of mutating client metadata directly.
///
/// Once a client has been promoted to a scratchpad, later protocol metadata
/// churn must not retag it back into a normal window.
pub fn handle_property_change(g: &mut CoreState, win: WindowId, props: &WindowProperties) -> bool {
    if let Some(c) = g.model.client_mut(win)
        && !props.title.is_empty()
    {
        c.name = props.title.clone();
    }

    if g.model.client(win).is_some_and(|c| c.scratchpad.is_some()) {
        return false;
    }

    let existing_context = g.model.client(win).map(|c| LaunchContext {
        monitor_id: c.monitor_id,
        tags: c.tags,
        is_floating: c.mode.is_floating(),
    });

    apply_rules(g, win, props, existing_context)
}

/// Apply a `RuleFloat` variant to `client`, optionally adjusting its geometry
/// using the supplied monitor geometry.
fn apply_float_rule(
    client: &mut crate::types::client::Client,
    float_rule: &RuleFloat,
    mon_geo: (Rect, Rect, bool),
    bar_height: i32,
) {
    let (monitor_rect, work_rect, show_bar) = mon_geo;

    match float_rule {
        RuleFloat::FloatCenter => {
            client.mode = ClientMode::Floating;
        }
        RuleFloat::FloatFullscreen => {
            client.mode = ClientMode::Floating;
            client.geo.w = monitor_rect.w;
            client.geo.h = work_rect.h;
            client.geo.x = monitor_rect.x;
            if show_bar {
                client.geo.y = monitor_rect.y + bar_height;
            }
        }
        RuleFloat::Scratchpad => {
            client.mode = ClientMode::Floating;
        }
        RuleFloat::Float => {
            client.mode = ClientMode::Floating;
            if show_bar {
                client.geo.y = monitor_rect.y + bar_height;
            }
        }
        RuleFloat::Tiled => {
            client.mode = ClientMode::Tiling;
        }
    }
}

/// Move `win` to the monitor named in `rule.monitor`, if any.
fn apply_monitor_rule(g: &mut CoreState, win: WindowId, rule: &crate::types::Rule) {
    let MonitorRule::Index(target_num) = rule.monitor else {
        return;
    };

    let target_mid = g
        .monitors_iter()
        .find(|(_i, m)| m.num == target_num as i32)
        .map(|(i, _)| i);

    if let Some(mid) = target_mid
        && let Some(c) = g.model.client_mut(win)
    {
        c.monitor_id = mid;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RuleStateSnapshot {
    is_floating: bool,
    is_sticky: bool,
    monitor_id: crate::types::MonitorId,
    tags: TagMask,
    geo: Rect,
}

fn rule_state_snapshot(g: &CoreState, win: WindowId) -> Option<RuleStateSnapshot> {
    let c = g.model.client(win)?;
    Some(RuleStateSnapshot {
        is_floating: c.mode.is_floating(),
        is_sticky: c.is_sticky,
        monitor_id: c.monitor_id,
        tags: c.tags,
        geo: c.geo,
    })
}

/// Clamp `win`'s tag mask to valid bits and fall back to the monitor's active
/// tags when no rule-assigned tag is currently visible.
fn clamp_client_tags(
    g: &mut CoreState,
    win: WindowId,
    tag_mask: TagMask,
    launch_context: Option<LaunchContext>,
) {
    let Some(view) = g.model.client_view(win) else {
        return;
    };
    let client_tags = view.client.tags;
    let monitor_tags = view.monitor.selected_tags();

    let mut final_tags = client_tags & tag_mask;
    if final_tags.is_empty() {
        final_tags = launch_context
            .map(|ctx| ctx.tags & tag_mask)
            .filter(|tags| !tags.is_empty())
            .unwrap_or(monitor_tags);
    }

    if let Some(c) = g.model.client_mut(win) {
        c.set_tag_mask(final_tags);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InitialRulePlacement, WindowProperties, apply_initial_rules, handle_property_change,
    };
    use crate::core_state::CoreState;
    use crate::types::{Client, ClientMode, Monitor, MonitorId, TagMask, WindowId};

    #[test]
    fn property_change_preserves_existing_tags_without_matching_rule() {
        let mut g = CoreState::default();
        g.model.tags.num_tags = 9;

        let mut mon = Monitor::new_with_values(true, true);
        mon.set_selected_tags(TagMask::single(1).unwrap());
        g.model.monitors.push(mon);

        let win = WindowId(42);
        let client = Client {
            win,
            monitor_id: MonitorId::default(),
            tags: TagMask::single(2).unwrap(),
            ..Default::default()
        };
        g.model.insert_client(client);

        handle_property_change(
            &mut g,
            win,
            &WindowProperties {
                title: "updated".to_string(),
                ..Default::default()
            },
        );

        let client = g.model.client(win).expect("client should still exist");
        assert_eq!(client.tags, TagMask::single(2).unwrap());
        assert_eq!(client.name, "updated");
    }

    #[test]
    fn property_change_preserves_manual_floating_state() {
        let mut g = CoreState::default();
        let win = WindowId(42);
        let client = Client {
            win,
            mode: ClientMode::Floating,
            ..Default::default()
        };
        g.model.insert_client(client);

        handle_property_change(
            &mut g,
            win,
            &WindowProperties {
                title: "updated".to_string(),
                ..Default::default()
            },
        );

        let client = g.model.client(win).expect("client should still exist");
        assert!(client.mode.is_floating());
    }

    #[test]
    fn rules_can_force_tiling_if_explicitly_specified() {
        use crate::types::{Monitor, MonitorRule, Rule, RuleFloat};
        use std::borrow::Cow;

        let mut g = CoreState::default();
        g.model.monitors.push(Monitor::new_with_values(true, true)); // Add a monitor

        g.config.bindings.rules = vec![Rule {
            class: Some(Cow::Borrowed("test")),
            instance: None,
            title: None,
            tags: TagMask::EMPTY,
            is_floating: Some(RuleFloat::Tiled), // Explicitly Tiled
            monitor: MonitorRule::Any,
        }];

        let win = WindowId(42);
        let client = Client {
            win,
            monitor_id: MonitorId::default(),
            mode: ClientMode::Floating,
            ..Default::default()
        };
        g.model.insert_client(client);

        handle_property_change(
            &mut g,
            win,
            &WindowProperties {
                class: "test".to_string(),
                title: "updated".to_string(),
                ..Default::default()
            },
        );

        let client = g.model.client(win).expect("client should still exist");
        assert!(!client.mode.is_floating()); // Should be tiling now
    }

    #[test]
    fn initial_float_center_rule_overrides_backend_position() {
        use crate::types::{MonitorRule, Rule, RuleFloat};
        use std::borrow::Cow;

        let mut g = CoreState::default();
        g.model.tags.num_tags = 1;
        let mut monitor = Monitor::new_with_values(true, true);
        monitor.set_selected_tags(TagMask::single(1).unwrap());
        g.model.monitors.push(monitor);
        g.config.bindings.rules = vec![Rule {
            class: Some(Cow::Borrowed("center-me")),
            instance: None,
            title: None,
            tags: TagMask::EMPTY,
            is_floating: Some(RuleFloat::FloatCenter),
            monitor: MonitorRule::Any,
        }];

        let win = WindowId(43);
        g.model.insert_client(Client {
            win,
            monitor_id: MonitorId::default(),
            ..Default::default()
        });

        let outcome = apply_initial_rules(
            &mut g,
            win,
            &WindowProperties {
                class: "center-me".to_string(),
                ..Default::default()
            },
            None,
        );

        assert_eq!(outcome.placement, InitialRulePlacement::Center);
        assert!(g.model.client(win).unwrap().mode.is_floating());
    }

    #[test]
    fn initial_fullscreen_float_rule_preserves_assigned_geometry() {
        use crate::types::{MonitorRule, Rect, Rule, RuleFloat};
        use std::borrow::Cow;

        let mut g = CoreState::default();
        g.model.tags.num_tags = 1;
        let mut monitor = Monitor::new_with_values(true, true);
        monitor.monitor_rect = Rect::new(1920, 0, 1920, 1080);
        monitor.work_rect = Rect::new(1920, 32, 1920, 1048);
        monitor.set_selected_tags(TagMask::single(1).unwrap());
        g.model.monitors.push(monitor);
        g.config.bindings.rules = vec![Rule {
            class: Some(Cow::Borrowed("fill-me")),
            instance: None,
            title: None,
            tags: TagMask::EMPTY,
            is_floating: Some(RuleFloat::FloatFullscreen),
            monitor: MonitorRule::Any,
        }];

        let win = WindowId(44);
        g.model.insert_client(Client {
            win,
            monitor_id: MonitorId::default(),
            ..Default::default()
        });

        let outcome = apply_initial_rules(
            &mut g,
            win,
            &WindowProperties {
                class: "fill-me".to_string(),
                ..Default::default()
            },
            None,
        );

        assert_eq!(outcome.placement, InitialRulePlacement::Preserve);
        assert_eq!(
            g.model.client(win).unwrap().geo,
            Rect::new(1920, 0, 1920, 1048)
        );
    }
}
