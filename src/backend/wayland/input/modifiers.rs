//! Translation between Smithay keyboard modifiers and the WM keybinding mask.

use smithay::input::keyboard::ModifiersState;

use crate::types::{ModMask, Modifier};

// ─────────────────────────────────────────────────────────────────────────────
// Input helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a Smithay `ModifiersState` to instantWM's modifier mask.
///
/// Wayland's `ModifiersState` is a struct of booleans, not a bitmask, and XKB's
/// own `ModMask` numbering is unrelated to the X11 `KeyButMask` numbering the
/// config format, the binding tables and the X11 backend all use. This is the
/// one place the two conventions meet.
pub fn modifiers_to_x11_mask(mods: &ModifiersState) -> ModMask {
    [
        (mods.shift, Modifier::Shift),
        (mods.ctrl, Modifier::Control),
        (mods.alt, Modifier::Alt),
        (mods.logo, Modifier::Super),
    ]
    .into_iter()
    .filter(|(held, _)| *held)
    .fold(ModMask::NONE, |mask, (_, modifier)| mask.with(modifier))
}
