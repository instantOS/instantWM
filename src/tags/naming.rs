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
    let state = &mut ctx.core_mut().state;
    if !rename_visible_tags(&mut state.model, &state.config.tag_template, arg) {
        return;
    }
    ctx.update_ewmh_desktop_props();
    ctx.request_bar_update();
}

fn rename_visible_tags(
    model: &mut crate::model::WmModel,
    configured: &[crate::types::Tag],
    arg: &str,
) -> bool {
    if arg.len() > MAX_TAG_NAME_BYTES {
        return false;
    }
    let monitor = model.expect_selected_monitor();
    let (num_tags, tagset) = (monitor.tags.len(), monitor.selected_tags().bits());
    if tagset == 0 {
        return false;
    }
    // Update the visible tags on every monitor, keeping session overrides in sync.
    for monitor in model.monitors.iter_all_mut() {
        for (index, tag) in monitor
            .tags
            .iter_mut()
            .take(num_tags.min(MAX_TAGS))
            .enumerate()
        {
            if tagset & (1 << index) != 0 {
                tag.name = if arg.is_empty() {
                    configured_tag_name(configured, index)
                } else {
                    arg.to_string()
                };
            }
        }
    }
    true
}

/// Reset every tag's name back to its configured label on all monitors.
pub fn reset_name_tag(ctx: &mut WmCtx) {
    let state = &mut ctx.core_mut().state;
    reset_tag_names(&mut state.model, &state.config.tag_template);
    ctx.update_ewmh_desktop_props();
    ctx.request_bar_update();
}

fn reset_tag_names(model: &mut crate::model::WmModel, configured: &[crate::types::Tag]) {
    let num_tags = model.tags.num_tags.min(MAX_TAGS);
    for monitor in model.monitors.iter_all_mut() {
        for (index, tag) in monitor.tags.iter_mut().take(num_tags).enumerate() {
            tag.name = configured_tag_name(configured, index);
        }
    }
}

fn configured_tag_name(configured: &[crate::types::Tag], index: usize) -> String {
    configured
        .get(index)
        .map(|tag| tag.name.clone())
        .unwrap_or_else(|| default_tag_name(index))
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
    use crate::config::config_toml::TagsConfig;
    use crate::config::resolve_config;
    use crate::test_support::TestWm as Wm;
    use crate::types::Monitor;

    fn wm_with(names: &[&str], icons: &[&str]) -> Wm {
        let mut user: crate::config::config_toml::UserConfig = toml::from_str("").unwrap();
        user.tags = TagsConfig {
            count: names.len(),
            names: names.iter().map(|n| n.to_string()).collect(),
            icons: icons.iter().map(|n| n.to_string()).collect(),
            show_icons: false,
        };
        let config = resolve_config(user, crate::backend::BackendKind::Wayland).unwrap();
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.model.monitors.push(Monitor::default());
        wm.core.state.apply_config(config).unwrap();
        wm.core
            .state
            .model
            .expect_selected_monitor_mut()
            .set_selected_tags(crate::types::TagMask::single(1).unwrap());
        wm
    }

    fn labels(wm: &Wm) -> Vec<String> {
        wm.core
            .state
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

        name_tag(&mut wm.test_ctx(), "browser");
        assert_eq!(labels(&wm), vec!["browser", "mail"]);

        name_tag(&mut wm.test_ctx(), "");
        assert_eq!(labels(&wm), vec!["web", "mail"]);
    }

    #[test]
    fn overlong_renames_are_ignored() {
        let mut wm = wm_with(&["web"], &[]);

        name_tag(&mut wm.test_ctx(), "this-name-is-far-too-long");
        assert_eq!(labels(&wm), vec!["web"]);
    }

    #[test]
    fn reset_restores_configured_names() {
        let mut wm = wm_with(&["web", "mail", "code"], &[]);
        name_tag(&mut wm.test_ctx(), "x");
        // Only tag 1 is selected, so tag 2 keeps its configured name.
        assert_eq!(labels(&wm), vec!["x", "mail", "code"]);

        reset_name_tag(&mut wm.test_ctx());
        assert_eq!(labels(&wm), vec!["web", "mail", "code"]);
    }
}
