// ---------------------------------------------------------------------------
// X11 Surface Classification Helpers
// ---------------------------------------------------------------------------

/// Classify an X11 surface as an "overlay" (override-redirect, popup, menu,
/// dmenu/instantmenu) at map time so we can cache the result and avoid
/// repeated string scans on every raise.
pub(crate) fn is_unmanaged_x11_overlay(x11: &smithay::xwayland::X11Surface) -> bool {
    if x11.is_override_redirect() || x11.is_popup() {
        return true;
    }
    if matches!(
        x11.window_type(),
        Some(
            smithay::xwayland::xwm::WmWindowType::DropdownMenu
                | smithay::xwayland::xwm::WmWindowType::Menu
                | smithay::xwayland::xwm::WmWindowType::PopupMenu
                | smithay::xwayland::xwm::WmWindowType::Tooltip
                | smithay::xwayland::xwm::WmWindowType::Notification
        )
    ) {
        return true;
    }
    is_launcher_x11_surface(x11)
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
