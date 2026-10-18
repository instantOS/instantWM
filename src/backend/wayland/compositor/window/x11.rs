// ---------------------------------------------------------------------------
// X11 Surface Classification Helpers
// ---------------------------------------------------------------------------

/// Classify an X11 surface as an "overlay" (override-redirect, popup, menu,
/// dmenu/instantmenu) at map time so we can cache the result and avoid
/// repeated string scans on every raise.
pub(crate) fn is_unmanaged_x11_overlay(x11: &smithay::xwayland::X11Surface) -> bool {
    // Modality describes interaction with a parent, not whether the window
    // should bypass management. Managed dialogs need a WindowId, transient
    // ancestry, and the normal map/click focus paths.
    unmanaged_overlay_policy(
        x11.is_override_redirect(),
        x11.window_type(),
        is_launcher_x11_surface(x11),
    )
}

fn unmanaged_overlay_policy(
    override_redirect: bool,
    window_type: Option<smithay::xwayland::xwm::WmWindowType>,
    launcher: bool,
) -> bool {
    override_redirect
        || launcher
        || matches!(
            window_type,
            Some(
                smithay::xwayland::xwm::WmWindowType::DropdownMenu
                    | smithay::xwayland::xwm::WmWindowType::Menu
                    | smithay::xwayland::xwm::WmWindowType::PopupMenu
                    | smithay::xwayland::xwm::WmWindowType::Tooltip
                    | smithay::xwayland::xwm::WmWindowType::Notification
            )
        )
}

#[cfg(test)]
mod tests {
    use super::unmanaged_overlay_policy;
    use smithay::xwayland::xwm::WmWindowType;

    #[test]
    fn dialogs_and_ordinary_windows_use_managed_focus_and_parent_tracking() {
        for typ in [None, Some(WmWindowType::Normal), Some(WmWindowType::Dialog)] {
            assert!(!unmanaged_overlay_policy(false, typ, false));
        }
    }

    #[test]
    fn popup_roles_remain_overlays() {
        for typ in [
            WmWindowType::DropdownMenu,
            WmWindowType::Menu,
            WmWindowType::PopupMenu,
            WmWindowType::Tooltip,
            WmWindowType::Notification,
        ] {
            assert!(unmanaged_overlay_policy(false, Some(typ), false));
        }
    }

    #[test]
    fn override_redirect_and_launchers_remain_unmanaged() {
        assert!(unmanaged_overlay_policy(
            true,
            Some(WmWindowType::Dialog),
            false
        ));
        assert!(unmanaged_overlay_policy(
            false,
            Some(WmWindowType::Normal),
            true
        ));
    }
}

/// Check if an X11 surface is a launcher (dmenu, instantmenu, etc.)
pub(crate) fn is_launcher_x11_surface(x11: &smithay::xwayland::X11Surface) -> bool {
    let class = x11.class().to_ascii_lowercase();
    let instance = x11.instance().to_ascii_lowercase();
    let title = x11.title().to_ascii_lowercase();
    class.contains("dmenu")
        || class.contains("instantmenu")
        || instance.contains("dmenu")
        || instance.contains("instantmenu")
        || title.contains("dmenu")
        || title.contains("instantmenu")
}
