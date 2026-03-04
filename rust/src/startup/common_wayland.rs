//! Common utilities shared between Wayland and DRM backends.

use smithay::input::keyboard::ModifiersState;

/// Extract font size from config font strings.
pub fn wayland_font_size_from_config(fonts: &[String]) -> f32 {
    fonts
        .iter()
        .find_map(|font| {
            let idx = font.find("size=")?;
            let tail = &font[idx + 5..];
            let num: String = tail
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            num.parse::<f32>().ok().filter(|s| *s > 0.0)
        })
        .unwrap_or(14.0)
}

/// Calculate font height from font size.
pub fn wayland_font_height_from_size(font_size: f32) -> i32 {
    ((font_size * 1.3).ceil() as i32).max(font_size.ceil() as i32 + 2)
}

/// Convert Smithay modifier state to X11-style mask.
pub fn modifiers_to_x11_mask(mods: &ModifiersState) -> u32 {
    let mut mask = 0u32;
    if mods.shift {
        mask |= crate::config::SHIFT;
    }
    if mods.ctrl {
        mask |= crate::config::CONTROL;
    }
    if mods.alt {
        mask |= crate::config::MOD1;
    }
    if mods.logo {
        mask |= crate::config::MODKEY;
    }
    mask
}
