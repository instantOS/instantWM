//! X11 keysyms and the constants instantWM binds by default.
//!
//! A *keysym* is the X11 protocol's name for "the symbol a key produces once the
//! keyboard layout has been applied". It is layout-dependent: on a US layout the
//! physical key labelled `Y` produces keysym `y`, and on a German layout it
//! produces `z`. Bindings are therefore written against keysyms, not against
//! physical key positions, and the same binding table works on every layout.
//!
//! The constant table below covers the keysyms instantWM's compiled defaults use,
//! plus a reference set so user configs can name exotic keys without looking up
//! hex values. Anything not listed can still be written by its XKB name
//! (`XF86AudioMute`, `dead_circumflex`, …) or as a single character.
//!
//! Not every constant is referenced in the default bindings. They are provided as
//! a complete reference so custom bindings can use any key.

use std::fmt;
use std::str::FromStr;

use smithay::input::keyboard::xkb;

/// An X11 keysym.
///
/// Wrapping the protocol's 32-bit keysym keeps keysyms from being confused with
/// any other `u32` that happens to be nearby, and gives name resolution and
/// binding normalization a home next to the value they operate on.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Keysym(u32);

impl Keysym {
    /// `NoSymbol`: the server reports no keysym for this key at this index.
    ///
    /// Distinct from "the key is unbound" — a keycode can legitimately map to
    /// nothing, and both the X11 grab scan and the modifier-key probe rely on
    /// being able to say so.
    pub const NONE: Keysym = Keysym::new(0);

    /// Wrap a raw X11 keysym value.
    ///
    /// This trusts the caller: the value is not checked for meaning. Use
    /// [`Keysym::from_name`] for anything that originated in user input.
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw X11 keysym value, for handing to XKB or X11 APIs.
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Fold this keysym into the form used to look up bindings.
    ///
    /// Shifting a letter produces its uppercase keysym (`XK_A_UPPER`), so
    /// `Super+Shift+a` and `Super+Shift+A` are the same chord. Both spellings
    /// must reach the same binding, so uppercase ASCII letters normalize down.
    /// Everything else is already its own base keysym.
    pub const fn for_binding(self) -> Self {
        // Written as explicit comparisons rather than `(b'A'..=b'Z').contains()`:
        // `RangeInclusive::contains` is not const-stable, and this runs on every
        // key press.
        if self.0 >= 0x41 && self.0 <= 0x5A {
            Self(self.0 + 0x20)
        } else {
            self
        }
    }

    /// Whether this keysym is a modifier key press.
    ///
    /// A modifier generates a key event of its own before the chord it takes
    /// part in arrives. Modal handlers that consume unmatched keys need to tell
    /// "the user is still assembling a chord" apart from "the user pressed
    /// something unbound", so they must recognize every modifier a configured
    /// chord could use — including `Alt` and `AltGr`, and not just the
    /// modifiers instantWM's own defaults happen to bind.
    pub const fn is_modifier(self) -> bool {
        // Matched on `self`, not `self.0`, so these are pattern alternatives
        // over `Keysym` constants rather than a bitwise-or on mismatched types.
        matches!(
            self,
            XK_SHIFT_L
                | XK_SHIFT_R
                | XK_CONTROL_L
                | XK_CONTROL_R
                | XK_ALT_L
                | XK_ALT_R
                | XK_SUPER_L
                | XK_SUPER_R
                | XK_ISO_LEVEL3_SHIFT
                | XK_ISO_LEVEL5_SHIFT
                | XK_MODE_SWITCH
        )
    }

    /// The keysym as a user would write it in a config file.
    ///
    /// Printable symbols render as the character itself, so `minus` reads back
    /// as `-`; everything else renders as its XKB name. This is the inverse of
    /// [`Keysym::from_name`] for every keysym that resolves, so a binding can be
    /// copied straight out of `instantwmctl keybinds` and back into a config.
    pub fn to_config_name(self) -> String {
        let text = xkb::keysym_to_utf8(xkb::Keysym::new(self.0));
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) if ch.is_ascii_graphic() => text,
            _ => xkb::keysym_get_name(xkb::Keysym::new(self.0)),
        }
    }

    /// Resolve a keysym from the name a user wrote in a config file.
    ///
    /// Accepts any XKB keysym name, case-insensitively (`Return`, `return`,
    /// `RETURN` all resolve to the same keysym), or a single character for
    /// punctuation keys (`-`, `/`, `+`).
    ///
    /// Names are resolved through XKB and nowhere else. instantWM deliberately
    /// keeps no alias table: an alias makes two spellings mean one thing, so a
    /// config can no longer be read back as the canonical form, and the tool
    /// that lists bindings would print a name the config never contained.
    pub fn from_name(name: &str) -> Result<Self, KeysymNameError> {
        let mut chars = name.chars();
        if let (Some(ch), None) = (chars.next(), chars.next())
            && !ch.is_alphanumeric()
        {
            let keysym = xkb::utf32_to_keysym(ch as u32).raw();
            if keysym != 0 {
                return Ok(Self(keysym));
            }
        }
        match xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE).raw() {
            0 => Err(KeysymNameError {
                name: name.to_string(),
            }),
            keysym => Ok(Self(keysym)),
        }
    }
}

impl FromStr for Keysym {
    type Err = KeysymNameError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::from_name(name)
    }
}

impl fmt::Display for Keysym {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_config_name())
    }
}

impl fmt::Debug for Keysym {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Keysym({} = {:#06X})", self.to_config_name(), self.0)
    }
}

/// A keysym name that XKB could not resolve.
///
/// Carries the offending name so the message names the thing the user actually
/// typed, and points at the name space that does resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeysymNameError {
    name: String,
}

impl fmt::Display for KeysymNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown key name '{}': expected an XKB keysym name such as \
             'Return', 'Escape', 'XF86AudioMute' or 'bracketleft', or a single \
             character such as '-'",
            self.name
        )
    }
}

impl std::error::Error for KeysymNameError {}

// ---------------------------------------------------------------------------
// Modifier keys
// ---------------------------------------------------------------------------
//
// A modifier key generates a key event of its own before the chord it takes part
// in arrives. `Keysym::is_modifier` exists so modal handlers can tell "still
// assembling a chord" apart from "pressed something unbound", which means the
// set has to cover every modifier a user-configured chord could name — not just
// the ones instantWM's compiled defaults happen to use.

pub const XK_SHIFT_L: Keysym = Keysym::new(0xFFE1);
pub const XK_SHIFT_R: Keysym = Keysym::new(0xFFE2);
pub const XK_CONTROL_L: Keysym = Keysym::new(0xFFE3);
pub const XK_CONTROL_R: Keysym = Keysym::new(0xFFE4);
pub const XK_CAPS_LOCK: Keysym = Keysym::new(0xFFE5);
pub const XK_ALT_L: Keysym = Keysym::new(0xFFE9);
pub const XK_ALT_R: Keysym = Keysym::new(0xFFEA);
pub const XK_SUPER_L: Keysym = Keysym::new(0xFFEB);
pub const XK_SUPER_R: Keysym = Keysym::new(0xFFEC);
/// Level-3 select. Usually AltGr on non-US layouts, which makes it the only way
/// to bind a key that layouts put outside the printable range.
pub const XK_ISO_LEVEL3_SHIFT: Keysym = Keysym::new(0xFE03);
/// Level-5 select.
pub const XK_ISO_LEVEL5_SHIFT: Keysym = Keysym::new(0xFE11);
/// X11's `Mode_switch`, a legacy alias some layouts and tools still emit.
pub const XK_MODE_SWITCH: Keysym = Keysym::new(0xFF7E);
/// X11's `Num_Lock`. Probed for by keycode to discover which modifier position
/// Num Lock currently occupies, since the server is free to map it anywhere.
pub const XK_NUM_LOCK: Keysym = Keysym::new(0xFF7F);

// ---------------------------------------------------------------------------
// Control / navigation
// ---------------------------------------------------------------------------

// Not an enum, deliberately. The keysym name space is open: a user config may
// name any of the ~2500 XKB keysyms, resolved at runtime through
// `Keysym::from_name`, and the values are sparse with high-bit XF86 ranges. A
// closed enum could not represent that, and the `Keysym` newtype already gives
// the type safety an enum would have: a keysym cannot be confused with any other
// `u32`, and it can only be built from a name that was actually resolved.

pub const XK_BACKSPACE: Keysym = Keysym::new(0xFF08);
pub const XK_TAB: Keysym = Keysym::new(0xFF09);
pub const XK_RETURN: Keysym = Keysym::new(0xFF0D);
pub const XK_ESCAPE: Keysym = Keysym::new(0xFF1B);
pub const XK_DELETE: Keysym = Keysym::new(0xFFFF);
pub const XK_HOME: Keysym = Keysym::new(0xFF50);
pub const XK_LEFT: Keysym = Keysym::new(0xFF51);
pub const XK_UP: Keysym = Keysym::new(0xFF52);
pub const XK_RIGHT: Keysym = Keysym::new(0xFF53);
pub const XK_DOWN: Keysym = Keysym::new(0xFF54);
pub const XK_PAGE_UP: Keysym = Keysym::new(0xFF55);
pub const XK_PAGE_DOWN: Keysym = Keysym::new(0xFF56);
pub const XK_END: Keysym = Keysym::new(0xFF57);
pub const XK_INSERT: Keysym = Keysym::new(0xFF63);

// a key event before the modified command arrives.

// ---------------------------------------------------------------------------
// Function keys
// ---------------------------------------------------------------------------

pub const XK_F1: Keysym = Keysym::new(0xFFBE);
pub const XK_F2: Keysym = Keysym::new(0xFFBF);
pub const XK_F3: Keysym = Keysym::new(0xFFC0);
pub const XK_F4: Keysym = Keysym::new(0xFFC1);
pub const XK_F5: Keysym = Keysym::new(0xFFC2);
pub const XK_F6: Keysym = Keysym::new(0xFFC3);
pub const XK_F7: Keysym = Keysym::new(0xFFC4);
pub const XK_F8: Keysym = Keysym::new(0xFFC5);
pub const XK_F9: Keysym = Keysym::new(0xFFC6);
pub const XK_F10: Keysym = Keysym::new(0xFFC7);
pub const XK_F11: Keysym = Keysym::new(0xFFC8);
pub const XK_F12: Keysym = Keysym::new(0xFFC9);

// ---------------------------------------------------------------------------
// Whitespace / misc printable
// ---------------------------------------------------------------------------

pub const XK_SPACE: Keysym = Keysym::new(0x0020);
pub const XK_EXCLAM: Keysym = Keysym::new(0x0021);
pub const XK_QUOTE_DBL: Keysym = Keysym::new(0x0022);
pub const XK_NUMBER_SIGN: Keysym = Keysym::new(0x0023);
pub const XK_DOLLAR: Keysym = Keysym::new(0x0024);
pub const XK_PERCENT: Keysym = Keysym::new(0x0025);
pub const XK_AMPERSAND: Keysym = Keysym::new(0x0026);
pub const XK_APOSTROPHE: Keysym = Keysym::new(0x0027);
pub const XK_PAREN_LEFT: Keysym = Keysym::new(0x0028);
pub const XK_PAREN_RIGHT: Keysym = Keysym::new(0x0029);
pub const XK_ASTERISK: Keysym = Keysym::new(0x002A);
pub const XK_PLUS: Keysym = Keysym::new(0x002B);
pub const XK_COMMA: Keysym = Keysym::new(0x002C);
pub const XK_MINUS: Keysym = Keysym::new(0x002D);
pub const XK_PERIOD: Keysym = Keysym::new(0x002E);
pub const XK_SLASH: Keysym = Keysym::new(0x002F);

// ---------------------------------------------------------------------------
// Digits
// ---------------------------------------------------------------------------

pub const XK_0: Keysym = Keysym::new(0x0030);
pub const XK_1: Keysym = Keysym::new(0x0031);
pub const XK_2: Keysym = Keysym::new(0x0032);
pub const XK_3: Keysym = Keysym::new(0x0033);
pub const XK_4: Keysym = Keysym::new(0x0034);
pub const XK_5: Keysym = Keysym::new(0x0035);
pub const XK_6: Keysym = Keysym::new(0x0036);
pub const XK_7: Keysym = Keysym::new(0x0037);
pub const XK_8: Keysym = Keysym::new(0x0038);
pub const XK_9: Keysym = Keysym::new(0x0039);

// ---------------------------------------------------------------------------
// Punctuation
// ---------------------------------------------------------------------------

pub const XK_COLON: Keysym = Keysym::new(0x003A);
pub const XK_SEMICOLON: Keysym = Keysym::new(0x003B);
pub const XK_LESS: Keysym = Keysym::new(0x003C);
pub const XK_EQUAL: Keysym = Keysym::new(0x003D);
pub const XK_GREATER: Keysym = Keysym::new(0x003E);
pub const XK_QUESTION: Keysym = Keysym::new(0x003F);
pub const XK_AT: Keysym = Keysym::new(0x0040);

// ---------------------------------------------------------------------------
// Uppercase letters  (Shift + key, or CapsLock)
// ---------------------------------------------------------------------------

pub const XK_A_UPPER: Keysym = Keysym::new(0x0041);
pub const XK_B_UPPER: Keysym = Keysym::new(0x0042);
pub const XK_C_UPPER: Keysym = Keysym::new(0x0043);
pub const XK_D_UPPER: Keysym = Keysym::new(0x0044);
pub const XK_E_UPPER: Keysym = Keysym::new(0x0045);
pub const XK_F_UPPER: Keysym = Keysym::new(0x0046);
pub const XK_G_UPPER: Keysym = Keysym::new(0x0047);
pub const XK_H_UPPER: Keysym = Keysym::new(0x0048);
pub const XK_I_UPPER: Keysym = Keysym::new(0x0049);
pub const XK_J_UPPER: Keysym = Keysym::new(0x004A);
pub const XK_K_UPPER: Keysym = Keysym::new(0x004B);
pub const XK_L_UPPER: Keysym = Keysym::new(0x004C);
pub const XK_M_UPPER: Keysym = Keysym::new(0x004D);
pub const XK_N_UPPER: Keysym = Keysym::new(0x004E);
pub const XK_O_UPPER: Keysym = Keysym::new(0x004F);
pub const XK_P_UPPER: Keysym = Keysym::new(0x0050);
pub const XK_Q_UPPER: Keysym = Keysym::new(0x0051);
pub const XK_R_UPPER: Keysym = Keysym::new(0x0052);
pub const XK_S_UPPER: Keysym = Keysym::new(0x0053);
pub const XK_T_UPPER: Keysym = Keysym::new(0x0054);
pub const XK_U_UPPER: Keysym = Keysym::new(0x0055);
pub const XK_V_UPPER: Keysym = Keysym::new(0x0056);
pub const XK_W_UPPER: Keysym = Keysym::new(0x0057);
pub const XK_X_UPPER: Keysym = Keysym::new(0x0058);
pub const XK_Y_UPPER: Keysym = Keysym::new(0x0059);
pub const XK_Z_UPPER: Keysym = Keysym::new(0x005A);

// ---------------------------------------------------------------------------
// Brackets / specials
// ---------------------------------------------------------------------------

pub const XK_BRACKET_LEFT: Keysym = Keysym::new(0x005B);
pub const XK_BACKSLASH: Keysym = Keysym::new(0x005C);
pub const XK_BRACKET_RIGHT: Keysym = Keysym::new(0x005D);
pub const XK_ASCII_CIRCUM: Keysym = Keysym::new(0x005E);
pub const XK_UNDERSCORE: Keysym = Keysym::new(0x005F);
pub const XK_GRAVE: Keysym = Keysym::new(0x0060);

// ---------------------------------------------------------------------------
// Lowercase letters  (unshifted letter keys)
// ---------------------------------------------------------------------------

pub const XK_A: Keysym = Keysym::new(0x0061);
pub const XK_B: Keysym = Keysym::new(0x0062);
pub const XK_C: Keysym = Keysym::new(0x0063);
pub const XK_D: Keysym = Keysym::new(0x0064);
pub const XK_E: Keysym = Keysym::new(0x0065);
pub const XK_F: Keysym = Keysym::new(0x0066);
pub const XK_G: Keysym = Keysym::new(0x0067);
pub const XK_H: Keysym = Keysym::new(0x0068);
pub const XK_I: Keysym = Keysym::new(0x0069);
pub const XK_J: Keysym = Keysym::new(0x006A);
pub const XK_K: Keysym = Keysym::new(0x006B);
pub const XK_L: Keysym = Keysym::new(0x006C);
pub const XK_M: Keysym = Keysym::new(0x006D);
pub const XK_N: Keysym = Keysym::new(0x006E);
pub const XK_O: Keysym = Keysym::new(0x006F);
pub const XK_P: Keysym = Keysym::new(0x0070);
pub const XK_Q: Keysym = Keysym::new(0x0071);
pub const XK_R: Keysym = Keysym::new(0x0072);
pub const XK_S: Keysym = Keysym::new(0x0073);
pub const XK_T: Keysym = Keysym::new(0x0074);
pub const XK_U: Keysym = Keysym::new(0x0075);
pub const XK_V: Keysym = Keysym::new(0x0076);
pub const XK_W: Keysym = Keysym::new(0x0077);
pub const XK_X: Keysym = Keysym::new(0x0078);
pub const XK_Y: Keysym = Keysym::new(0x0079);
pub const XK_Z: Keysym = Keysym::new(0x007A);

// ---------------------------------------------------------------------------
// Special keys
// ---------------------------------------------------------------------------

pub const XK_PRINT: Keysym = Keysym::new(0xFF61);
/// Dead key: combining circumflex accent (^).
pub const XK_DEAD_CIRCUMFLEX: Keysym = Keysym::new(0xFE52);

// ---------------------------------------------------------------------------
// XF86 media / hardware keys
// ---------------------------------------------------------------------------

pub const XF86XK_MON_BRIGHTNESS_UP: Keysym = Keysym::new(0x1008FF02);
pub const XF86XK_MON_BRIGHTNESS_DOWN: Keysym = Keysym::new(0x1008FF03);
pub const XF86XK_AUDIO_LOWER_VOLUME: Keysym = Keysym::new(0x1008FF11);
pub const XF86XK_AUDIO_MUTE: Keysym = Keysym::new(0x1008FF12);
pub const XF86XK_AUDIO_MIC_MUTE: Keysym = Keysym::new(0x1008FFB2);
pub const XF86XK_AUDIO_RAISE_VOLUME: Keysym = Keysym::new(0x1008FF13);
pub const XF86XK_AUDIO_PLAY: Keysym = Keysym::new(0x1008FF14);
pub const XF86XK_AUDIO_PAUSE: Keysym = Keysym::new(0x1008FF15);
pub const XF86XK_AUDIO_NEXT: Keysym = Keysym::new(0x1008FF17);
pub const XF86XK_AUDIO_PREV: Keysym = Keysym::new(0x1008FF16);

// ---------------------------------------------------------------------------
// XF86 VT switch keys
// ---------------------------------------------------------------------------

pub const XF86XK_SWITCH_VT_1: Keysym = Keysym::new(0x1008FE01);
pub const XF86XK_SWITCH_VT_2: Keysym = Keysym::new(0x1008FE02);
pub const XF86XK_SWITCH_VT_3: Keysym = Keysym::new(0x1008FE03);
pub const XF86XK_SWITCH_VT_4: Keysym = Keysym::new(0x1008FE04);
pub const XF86XK_SWITCH_VT_5: Keysym = Keysym::new(0x1008FE05);
pub const XF86XK_SWITCH_VT_6: Keysym = Keysym::new(0x1008FE06);
pub const XF86XK_SWITCH_VT_7: Keysym = Keysym::new(0x1008FE07);
pub const XF86XK_SWITCH_VT_8: Keysym = Keysym::new(0x1008FE08);
pub const XF86XK_SWITCH_VT_9: Keysym = Keysym::new(0x1008FE09);
pub const XF86XK_SWITCH_VT_10: Keysym = Keysym::new(0x1008FE0A);
pub const XF86XK_SWITCH_VT_11: Keysym = Keysym::new(0x1008FE0B);
pub const XF86XK_SWITCH_VT_12: Keysym = Keysym::new(0x1008FE0C);
