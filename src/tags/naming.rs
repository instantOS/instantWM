//! Tag name management.
//!
//! Configured labels (see [`crate::config::config_toml::TagsConfig`]) define
//! the defaults; `instantwmctl tag name` and `tag reset` are session
//! overrides on top of them, dropped by `reload`.

use crate::contexts::WmCtx;
use crate::types::MAX_TAGS;
use crate::types::tag::MAX_TAG_NAME_BYTES;

/// Rename the currently visible tag(s) on the selected monitor.
///
/// An empty `arg` restores the configured label for each affected tag.
/// Names longer than [`MAX_TAG_NAME_BYTES`] bytes are silently ignored.
///
/// All tags included in the monitor's current tagset are renamed, so the
/// function works correctly even when multiple tags are visible at once.
pub fn name_tag(ctx: &mut WmCtx, arg: &str) {
    if arg.len() > MAX_TAG_NAME_BYTES {
        return;
    }

    let mon = ctx.core().model().expect_selected_monitor();
    let (num_tags, tagset) = (mon.tags.len(), mon.selected_tags().bits());

    if tagset == 0 {
        return;
    }

    let configured = ctx.core().config().tag_template.clone();
    let label_for = |index: usize| -> String {
        if arg.is_empty() {
            configured
                .get(index)
                .map(|tag| tag.name.clone())
                .unwrap_or_else(|| default_tag_name(index))
        } else {
            arg.to_string()
        }
    };

    // Apply the new label to every tag in the current tagset on every
    // monitor, so secondary monitors stay in sync.
    for mon in ctx.core_mut().model_mut().monitors.iter_all_mut() {
        for (i, tag) in mon.tags.iter_mut().take(num_tags.min(MAX_TAGS)).enumerate() {
            if (tagset & (1 << i)) == 0 {
                continue;
            }
            tag.name = label_for(i);
        }
    }

    ctx.update_ewmh_desktop_props();
    ctx.request_bar_update();
}

/// Reset every tag's name back to its configured label on all monitors.
//BOZO: should there maybe be a Tag struct which this is a method of? Is there
//already such a struct maybe? Same goes for many of the functions in this file
pub fn reset_name_tag(ctx: &mut WmCtx) {
    let configured = ctx.core().config().tag_template.clone();
    let num_tags = ctx.core().model().tags.num_tags.min(MAX_TAGS);
    for mon in ctx.core_mut().model_mut().monitors.iter_all_mut() {
        for (i, tag) in mon.tags.iter_mut().take(num_tags).enumerate() {
            tag.name = configured
                .get(i)
                .map(|tag| tag.name.clone())
                .unwrap_or_else(|| default_tag_name(i));
        }
    }

    ctx.update_ewmh_desktop_props();
    ctx.request_bar_update();
}

/// Fallback label for tag index `i` (0-based) when no configured label
/// exists for it.
///
fn default_tag_name(i: usize) -> String {
    (i + 1).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Backend, wayland::WaylandBackend};
    use crate::config::config_toml::TagsConfig;
    use crate::config::resolve_config;
    use crate::types::Monitor;
    use crate::wm::Wm;

    fn wm_with(names: &[&str], icons: &[&str]) -> Wm {
        let mut user: crate::config::config_toml::UserConfig = toml::from_str("").unwrap();
        user.tags = TagsConfig {
            count: names.len(),
            names: names.iter().map(|n| n.to_string()).collect(),
            icons: icons.iter().map(|n| n.to_string()).collect(),
            show_icons: false,
        };
        let config = resolve_config(user, crate::backend::BackendKind::Wayland).unwrap();
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.model.monitors.push(Monitor::default());
        wm.core.apply_config(config).unwrap();
        wm.core
            .model
            .expect_selected_monitor_mut()
            .set_selected_tags(crate::types::TagMask::single(1).unwrap());
        wm
    }

    fn labels(wm: &Wm) -> Vec<String> {
        wm.core
            .model
            .expect_selected_monitor()
            .tags
            .iter()
            .map(|tag| tag.name.clone())
            .collect()
    }

    #[test]
    fn rename_is_a_session_override_and_empty_arg_restores_the_configured_name() {
        let mut wm = wm_with(&["web", "mail"], &["W", "M"]);

        name_tag(&mut wm.ctx(), "browser");
        assert_eq!(labels(&wm), vec!["browser", "mail"]);

        name_tag(&mut wm.ctx(), "");
        assert_eq!(labels(&wm), vec!["web", "mail"]);
    }

    #[test]
    fn overlong_renames_are_ignored() {
        let mut wm = wm_with(&["web"], &[]);

        name_tag(&mut wm.ctx(), "this-name-is-far-too-long");
        assert_eq!(labels(&wm), vec!["web"]);
    }

    #[test]
    fn reset_restores_configured_names() {
        let mut wm = wm_with(&["web", "mail", "code"], &[]);
        name_tag(&mut wm.ctx(), "x");
        // Only tag 1 is selected, so tag 2 keeps its configured name.
        assert_eq!(labels(&wm), vec!["x", "mail", "code"]);

        reset_name_tag(&mut wm.ctx());
        assert_eq!(labels(&wm), vec!["web", "mail", "code"]);
    }
}
