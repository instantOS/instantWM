//! Runtime colour-theme get/set/list over IPC.
//!
//! Setting a theme recomputes the full colour tables from the built-in palette
//! and pushes them to the bar/borders. Like other `instantwmctl` runtime
//! changes, this is non-persistent: `reload` reverts to whatever `config.toml`
//! contains.

use crate::config::config_toml::ColorTheme;
use crate::config::runtime::ConfigEffect;
use crate::ipc_types::Response;

/// Return the name of the active theme.
pub fn get_theme(ctx: &crate::contexts::WmCtx<'_>) -> Response {
    Response::Theme(ctx.core().config().theme.name())
}

/// List every built-in theme name.
pub fn list_themes() -> Response {
    Response::ThemeList(
        <ColorTheme as clap::ValueEnum>::value_variants()
            .iter()
            .map(|theme| theme.name())
            .collect(),
    )
}

/// Switch to a built-in theme, recolouring the running WM.
pub fn set_theme(ctx: &mut crate::contexts::WmCtx<'_>, theme: ColorTheme) -> Response {
    // Recompute every colour table from the theme palette and install it as
    // one unit. Per-monitor tag sets mirror the shared tag table.
    let colors = crate::config::appearance::ColorConfig::from(theme);
    ctx.core_mut().model_mut().tags.colors = colors.tag.clone();
    ctx.core_mut().config_mut().colors = colors;
    ctx.core_mut().config_mut().theme = theme;
    crate::actions::apply_config_effect(ctx, ConfigEffect::Recolor);
    Response::ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestWm as Wm;

    fn test_wm() -> Wm {
        Wm::new(crate::backend::WaylandBackendData::default())
    }

    #[test]
    fn set_theme_recolors_both_color_stores_and_records_it() {
        let mut wm = test_wm();
        assert!(matches!(
            wm.with_ctx(|wm| set_theme(wm, ColorTheme::Nord)),
            Response::Ok
        ));

        assert_eq!(wm.core.state.config.theme, ColorTheme::Nord);
        // Tag colours live in `model.tags.colors`…
        let nord = crate::config::appearance::ColorConfig::from(ColorTheme::Nord);
        assert_eq!(
            wm.core.state.model.tags.colors.no_hover.focus.background,
            nord.tag.no_hover.focus.background
        );
        // …the rest in `config.colors`.
        assert_eq!(
            wm.core.state.config.colors.border.tile_focus,
            nord.border.tile_focus
        );
        assert_eq!(
            wm.core.state.config.colors.status.foreground,
            nord.status.foreground
        );
    }

    #[test]
    fn get_theme_returns_the_active_name() {
        let mut wm = test_wm();
        wm.with_ctx(|wm| set_theme(wm, ColorTheme::Gruvbox));
        match get_theme(&wm.test_ctx()) {
            Response::Theme(name) => assert_eq!(name, "gruvbox"),
            other => panic!("expected Theme, got {other:?}"),
        }
    }

    #[test]
    fn list_themes_returns_every_name() {
        match list_themes() {
            Response::ThemeList(names) => {
                assert_eq!(
                    names.len(),
                    <ColorTheme as clap::ValueEnum>::value_variants().len()
                );
                assert!(names.contains(&"nord".to_string()));
                assert!(names.contains(&"catppuccin-mocha".to_string()));
            }
            other => panic!("expected ThemeList, got {other:?}"),
        }
    }
}
