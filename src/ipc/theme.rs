//! Runtime colour-theme get/set/list over IPC.
//!
//! Setting a theme recomputes the full colour tables from the built-in palette
//! and pushes them to the bar/borders. Like other `instantwmctl` runtime
//! changes, this is non-persistent: `reload` reverts to whatever `config.toml`
//! contains.

use crate::config::appearance;
use crate::config::config_toml::ColorTheme;
use crate::ipc::config;
use crate::ipc_types::Response;
use crate::wm::Wm;

/// Return the name of the active theme.
pub fn get_theme(wm: &Wm) -> Response {
    Response::Theme(wm.core.config.theme.name())
}

/// List every built-in theme name.
pub fn list_themes() -> Response {
    Response::ThemeList(ColorTheme::ALL.iter().map(|t| t.name()).collect())
}

/// Switch to a built-in theme, recolouring the running WM.
pub fn set_theme(wm: &mut Wm, theme: ColorTheme) -> Response {
    // Recompute every colour table from the theme palette, then drop it onto
    // the runtime colour state. Tag colours live separately from the rest, so
    // both stores are updated.
    let colors = appearance::colors(theme);
    wm.core.config.colors.window = colors.window;
    wm.core.config.colors.close_button = colors.close_button;
    wm.core.config.colors.border = colors.border;
    wm.core.config.colors.status_bar = colors.status;
    wm.core.model.tags.colors = colors.tag;
    wm.core.config.theme = theme;
    config::recolor(wm);
    Response::ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Backend, wayland::WaylandBackend};

    fn test_wm() -> Wm {
        Wm::new(Backend::new_wayland(WaylandBackend::new()))
    }

    #[test]
    fn set_theme_recolors_both_color_stores_and_records_it() {
        let mut wm = test_wm();
        assert!(matches!(set_theme(&mut wm, ColorTheme::Nord), Response::Ok));

        assert_eq!(wm.core.config.theme, ColorTheme::Nord);
        // Tag colours live in `model.tags.colors`…
        let nord = appearance::colors(ColorTheme::Nord);
        assert_eq!(
            wm.core.model.tags.colors.no_hover.focus.bg,
            nord.tag.no_hover.focus.bg
        );
        // …the rest in `config.colors`.
        assert_eq!(
            wm.core.config.colors.border.tile_focus,
            nord.border.tile_focus
        );
        assert_eq!(wm.core.config.colors.status_bar.fg, nord.status.fg);
    }

    #[test]
    fn get_theme_returns_the_active_name() {
        let mut wm = test_wm();
        set_theme(&mut wm, ColorTheme::Gruvbox);
        match get_theme(&wm) {
            Response::Theme(name) => assert_eq!(name, "gruvbox"),
            other => panic!("expected Theme, got {other:?}"),
        }
    }

    #[test]
    fn list_themes_returns_every_name() {
        match list_themes() {
            Response::ThemeList(names) => {
                assert_eq!(names.len(), ColorTheme::ALL.len());
                assert!(names.contains(&"nord".to_string()));
                assert!(names.contains(&"catppuccin-mocha".to_string()));
            }
            other => panic!("expected ThemeList, got {other:?}"),
        }
    }
}
