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
    override_redirect || launcher || is_overlay_window_type(window_type)
}

pub(super) fn is_overlay_window_type(
    window_type: Option<smithay::xwayland::xwm::WmWindowType>,
) -> bool {
    matches!(
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
    use super::{is_launcher_name, unmanaged_overlay_policy};
    use smithay::xwayland::xwm::WmWindowType;

    #[test]
    fn launcher_matching_preserves_ascii_case_and_substring_semantics() {
        for name in [
            "dmenu",
            "DMenu",
            "InstantMenu",
            "prefix-INSTANTMENU-suffix",
            "αDMENUβ",
        ] {
            assert!(is_launcher_name(name), "{name}");
        }
        for name in ["", "instantWM", "d menu", "DMEN", "Wine game"] {
            assert!(!is_launcher_name(name), "{name}");
        }
    }

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
    is_launcher_name(&x11.class())
        || is_launcher_name(&x11.instance())
        || is_launcher_name(&x11.title())
}

fn is_launcher_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes
        .windows(b"dmenu".len())
        .any(|part| part.eq_ignore_ascii_case(b"dmenu"))
        || bytes
            .windows(b"instantmenu".len())
            .any(|part| part.eq_ignore_ascii_case(b"instantmenu"))
}
