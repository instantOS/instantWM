//! Cursor presentation policy shared by nested and DRM renderers.

use smithay::input::pointer::{CursorIcon, CursorImageAttributes, CursorImageStatus};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::with_states;
use std::sync::Mutex;

/// Backend-agnostic cursor state after applying WM override policy.
#[derive(Debug, PartialEq)]
pub enum CursorPresentation {
    Hidden,
    Named(CursorIcon),
    Surface {
        surface: WlSurface,
        hotspot: Point<i32, Logical>,
    },
    DndIcon {
        icon: WlSurface,
        hotspot: Point<i32, Logical>,
        cursor: Box<CursorPresentation>,
    },
}

/// Resolve effective cursor state shared by nested and DRM backends.
///
/// WM icon overrides are only visual hints for compositor-driven interactions.
/// A client-hidden cursor must remain hidden so relative pointer users, such as
/// games running through XWayland, cannot be defeated by stale hover state.
pub fn resolve_cursor_presentation(
    status: &CursorImageStatus,
    icon_override: Option<CursorIcon>,
    dnd_icon: Option<&WlSurface>,
    hidden_by_touch: bool,
) -> CursorPresentation {
    if hidden_by_touch {
        return CursorPresentation::Hidden;
    }
    let base = match status {
        CursorImageStatus::Hidden => CursorPresentation::Hidden,
        CursorImageStatus::Named(icon) => CursorPresentation::Named(icon_override.unwrap_or(*icon)),
        CursorImageStatus::Surface(surface) => {
            if let Some(icon) = icon_override {
                CursorPresentation::Named(icon)
            } else {
                // Check if the cursor surface is still alive before using it.
                // If the surface is dead, fall back to the default cursor icon.
                if !smithay::utils::IsAlive::alive(surface) {
                    return CursorPresentation::Named(CursorIcon::Default);
                }
                let hotspot = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<Mutex<CursorImageAttributes>>()
                        .and_then(|attrs| attrs.lock().ok().map(|guard| guard.hotspot))
                        .unwrap_or((0, 0).into())
                });
                CursorPresentation::Surface {
                    surface: surface.clone(),
                    hotspot,
                }
            }
        }
    };

    if let Some(icon) = dnd_icon
        && smithay::utils::IsAlive::alive(icon)
    {
        let hotspot = with_states(icon, |states| {
            states
                .data_map
                .get::<Mutex<CursorImageAttributes>>()
                .and_then(|attrs| attrs.lock().ok().map(|guard| guard.hotspot))
                .unwrap_or((0, 0).into())
        });
        return CursorPresentation::DndIcon {
            icon: icon.clone(),
            hotspot,
            cursor: Box::new(base),
        };
    }

    base
}

#[cfg(test)]
mod tests {
    use smithay::input::pointer::{CursorIcon, CursorImageStatus};

    use super::{CursorPresentation, resolve_cursor_presentation};

    #[test]
    fn hidden_cursor_status_wins_over_wm_icon_override() {
        let presentation = resolve_cursor_presentation(
            &CursorImageStatus::Hidden,
            Some(CursorIcon::Grabbing),
            None,
            false,
        );

        assert!(matches!(presentation, CursorPresentation::Hidden));
    }

    #[test]
    fn wm_icon_override_still_applies_to_named_cursor_status() {
        let presentation = resolve_cursor_presentation(
            &CursorImageStatus::Named(CursorIcon::Default),
            Some(CursorIcon::Grabbing),
            None,
            false,
        );

        assert_eq!(
            presentation,
            CursorPresentation::Named(CursorIcon::Grabbing)
        );
    }
}
