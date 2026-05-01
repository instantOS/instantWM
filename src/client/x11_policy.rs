use smithay::xwayland::{X11Surface, xwm::WmWindowType};
use x11rb::properties::{WmHints, WmSizeHints};

use crate::client::rules::WindowProperties;
use crate::types::{Client, WindowId};

pub fn window_properties_from_x11_surface(surface: &X11Surface) -> WindowProperties {
    WindowProperties {
        class: surface.class(),
        instance: surface.instance(),
        title: surface.title(),
    }
}

pub fn transient_for_window_id(surface: &X11Surface) -> Option<WindowId> {
    surface.is_transient_for().map(WindowId::from)
}

pub fn apply_wm_hints_to_client(client: &mut Client, hints: Option<WmHints>) {
    let (never_focus, is_urgent) = hints
        .map(|hints| (!hints.input.unwrap_or(true), hints.urgent))
        .unwrap_or((false, false));
    client.never_focus = never_focus;
    client.is_urgent = is_urgent;
}

pub fn apply_size_hints_to_client(client: &mut Client, hints: Option<WmSizeHints>) {
    client.size_hints = Default::default();
    client.min_aspect = 0.0;
    client.max_aspect = 0.0;

    if let Some(hints) = hints {
        let base_size = hints.base_size.or(hints.min_size).unwrap_or((0, 0));
        client.size_hints.basew = base_size.0;
        client.size_hints.baseh = base_size.1;

        let min_size = hints.min_size.or(hints.base_size).unwrap_or((0, 0));
        client.size_hints.minw = min_size.0;
        client.size_hints.minh = min_size.1;

        let max_size = hints.max_size.unwrap_or((0, 0));
        client.size_hints.maxw = max_size.0;
        client.size_hints.maxh = max_size.1;

        let increments = hints.size_increment.unwrap_or((0, 0));
        client.size_hints.incw = increments.0;
        client.size_hints.inch = increments.1;

        if let Some((min_aspect, max_aspect)) = hints.aspect {
            if min_aspect.denominator != 0 {
                client.min_aspect = min_aspect.numerator as f32 / min_aspect.denominator as f32;
            }
            if max_aspect.denominator != 0 {
                client.max_aspect = max_aspect.numerator as f32 / max_aspect.denominator as f32;
            }
        }
    }

    client.is_fixed_size = client.size_hints.is_fixed();
    client.size_hints_dirty = true;
}

pub fn should_float_for_x11_type(window_type: Option<WmWindowType>) -> bool {
    matches!(
        window_type,
        Some(
            WmWindowType::Dialog
                | WmWindowType::Utility
                | WmWindowType::Toolbar
                | WmWindowType::Splash
        )
    )
}

pub fn preferred_border_width(borderpx: i32, decorated: bool) -> i32 {
    if decorated { 0 } else { borderpx }
}
