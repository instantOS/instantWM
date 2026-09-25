//! Keyboard modifier masks.
//!
//! instantWM encodes modifiers as X11-protocol `KeyButMask` bits. That choice is
//! deliberate and backend-independent: the config format, the keybind tables and
//! the mouse bindings all speak X11 modifier bits, so a config means the same
//! thing on either backend. Wayland's `ModifiersState` is not a bitmask, so the
//! Wayland backend converts it once, at the edge, in
//! `backend::wayland::input::modifiers_to_x11_mask`.
//!
//! There is exactly one convention here, and these two types are the only place
//! it is written down. [`Modifier`] is a single bit; [`ModMask`] is a set of
//! them.

use std::fmt;
use std::ops::{BitAnd, BitOr, BitOrAssign};
use std::str::FromStr;

/// A single keyboard modifier.
///
/// The bit positions are the X11 wire positions and are fixed by the protocol.
/// The *names* follow one rule: a variant gets a semantic name when the X11
/// protocol and every real server agree on what the bit is, and a positional
/// name when the meaning is deployment-specific.
///
/// - `Shift`, `Control`, `Alt` and `Super` are pinned by the protocol on every
///   server, so they are named for what they do.
/// - `Mod2`, `Mod3` and `Mod5` have no universal meaning. `Mod2` is usually
///   Num Lock, `Mod3` is often level-3 select, and `Mod5` is usually AltGr, but
///   all three are reassignable and sometimes unbound. Naming them after a
///   guessed purpose would be a lie that misleads exactly the users who
///   configured the unusual case, so they are named by position.
///
/// `Super` is bit 6 (`Mod4Mask`) because instantWM hard-codes its primary
/// modifier there. It is the WM's own choice, not something the protocol
/// dictates.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Modifier {
    /// Shift. Bit 0.
    Shift,
    /// Caps Lock. Bit 1, the X11 `LockMask` position.
    CapsLock,
    /// Control. Bit 2.
    Control,
    /// Alt. Bit 3, the X11 `Mod1Mask` position.
    Alt,
    /// Bit 4 (`Mod2Mask`). Meaning is deployment-specific.
    Mod2,
    /// Bit 5 (`Mod3Mask`). Meaning is deployment-specific.
    Mod3,
    /// Super (Windows / Command). Bit 6, the X11 `Mod4Mask` position.
    Super,
    /// Bit 7 (`Mod5Mask`). Meaning is deployment-specific.
    Mod5,
}

impl Modifier {
    /// Every modifier, in the order [`ModMask`] renders and iterates them.
    ///
    /// This is presentation order, not bit order: instantWM's primary modifier
    /// leads, then the modifiers people reach for next. Rendering and iteration
    /// share it deliberately, so anything that collects a mask into a list and
    /// anything that renders one cannot disagree about ordering.
    pub const DISPLAY_ORDER: [Modifier; 8] = [
        Modifier::Super,
        Modifier::Control,
        Modifier::Shift,
        Modifier::Alt,
        Modifier::Mod2,
        Modifier::Mod3,
        Modifier::Mod5,
        Modifier::CapsLock,
    ];

    /// The X11 `KeyButMask` bit for this modifier.
    pub const fn bit(self) -> u16 {
        match self {
            Modifier::Shift => 0b0000_0001,
            Modifier::CapsLock => 0b0000_0010,
            Modifier::Control => 0b0000_0100,
            Modifier::Alt => 0b0000_1000,
            Modifier::Mod2 => 0b0001_0000,
            Modifier::Mod3 => 0b0010_0000,
            Modifier::Super => 0b0100_0000,
            Modifier::Mod5 => 0b1000_0000,
        }
    }

    /// The modifier occupying `bit`, if that position is a modifier instantWM
    /// models.
    pub const fn from_bit(bit: u16) -> Option<Self> {
        // Binary literals rather than `1 << n`: shifts are expressions, not
        // patterns, and spelling the bit out keeps the wire layout readable.
        match bit {
            0b0000_0001 => Some(Modifier::Shift),
            0b0000_0010 => Some(Modifier::CapsLock),
            0b0000_0100 => Some(Modifier::Control),
            0b0000_1000 => Some(Modifier::Alt),
            0b0001_0000 => Some(Modifier::Mod2),
            0b0010_0000 => Some(Modifier::Mod3),
            0b0100_0000 => Some(Modifier::Super),
            0b1000_0000 => Some(Modifier::Mod5),
            _ => None,
        }
    }

    /// The positional name the i3bar click-event protocol uses for this bit.
    ///
    /// instantWM inherits the i3bar protocol verbatim and serializes click
    /// modifier names onto the bar's stdin, where user scripts read them. That
    /// makes these strings an external contract with a fixed vocabulary, and it
    /// is deliberately *not* the same vocabulary as [`Modifier`]'s own `Display`.
    /// Do not "unify" the two: renaming these breaks every user script that
    /// compares against `Mod4`.
    pub const fn i3_name(self) -> &'static str {
        match self {
            Modifier::Shift => "Shift",
            Modifier::CapsLock => "Lock",
            Modifier::Control => "Control",
            Modifier::Alt => "Mod1",
            Modifier::Mod2 => "Mod2",
            Modifier::Mod3 => "Mod3",
            Modifier::Super => "Mod4",
            Modifier::Mod5 => "Mod5",
        }
    }
}

impl fmt::Display for Modifier {
    /// The canonical config spelling, in title case.
    ///
    /// [`Keysym`](crate::types::Keysym) parses names case-insensitively, so this
    /// output round-trips straight back into a config file. There is exactly one
    /// spelling per modifier: instantWM keeps no alias table, because an alias
    /// means two spellings resolve to one thing, and a config can then no longer
    /// be read back as the form the tooling prints.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Modifier::Shift => "Shift",
            Modifier::CapsLock => "CapsLock",
            Modifier::Control => "Control",
            Modifier::Alt => "Alt",
            Modifier::Mod2 => "Mod2",
            Modifier::Mod3 => "Mod3",
            Modifier::Super => "Super",
            Modifier::Mod5 => "Mod5",
        })
    }
}

impl FromStr for Modifier {
    type Err = ModifierNameError;

    /// Parse a modifier from its canonical name, case-insensitively.
    ///
    /// One name per modifier. `Super` is bit 6, and `Mod4` is not accepted for
    /// it: accepting the positional spelling alongside the semantic one would
    /// make the config language ambiguous about which vocabulary it speaks.
    /// The positional names that *are* canonical — `Mod2`, `Mod3`, `Mod5` — are
    /// the ones with no universal meaning to name semantically.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name.to_ascii_lowercase().as_str() {
            "shift" => Ok(Modifier::Shift),
            "capslock" => Ok(Modifier::CapsLock),
            "control" => Ok(Modifier::Control),
            "alt" => Ok(Modifier::Alt),
            "mod2" => Ok(Modifier::Mod2),
            "mod3" => Ok(Modifier::Mod3),
            "super" => Ok(Modifier::Super),
            "mod5" => Ok(Modifier::Mod5),
            _ => Err(ModifierNameError {
                name: name.to_string(),
            }),
        }
    }
}

impl fmt::Debug for Modifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self} (bit {})", self.bit())
    }
}

/// A modifier name that instantWM does not recognize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModifierNameError {
    name: String,
}

impl fmt::Display for ModifierNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown modifier '{}': expected one of {}",
            self.name,
            Modifier::DISPLAY_ORDER
                .into_iter()
                .map(|m| m.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl std::error::Error for ModifierNameError {}

/// A set of keyboard modifiers.
///
/// The backing integer is `u16` because that is exactly the X11 `ModMask` wire
/// width, and every modifier instantWM models occupies one of its 8 bits. Using
/// the protocol's own width means the masks that reach the X11 backend need no
/// narrowing cast, so a mask cannot be silently truncated on its way to a
/// `GrabKey` request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ModMask(u16);

impl ModMask {
    /// No modifiers held.
    pub const NONE: ModMask = ModMask(0);

    /// Wrap a raw X11 modifier mask.
    ///
    /// Bits outside the 8 modifier positions are preserved, so a mask straight
    /// off the wire survives a round trip. Use [`ModMask::cleaned`] to drop the
    /// lock modifiers that should not participate in binding matching.
    pub const fn new(bits: u16) -> Self {
        Self(bits)
    }

    /// A mask holding exactly `modifier`.
    pub const fn from_modifier(modifier: Modifier) -> Self {
        Self(modifier.bit())
    }

    /// The raw X11 `ModMask` bits.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Whether no modifier is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Set-union of two masks, usable in `const` context.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether `modifier` is held.
    pub const fn contains(self, modifier: Modifier) -> bool {
        self.0 & modifier.bit() != 0
    }

    /// This mask with `modifier` set.
    pub const fn with(self, modifier: Modifier) -> Self {
        Self(self.0 | modifier.bit())
    }

    /// This mask with `modifier` cleared.
    pub const fn without(self, modifier: Modifier) -> Self {
        Self(self.0 & !modifier.bit())
    }

    /// Drop the lock modifiers that must not affect binding matching.
    ///
    /// Caps Lock and Num Lock are sticky: they stay held while the user types,
    /// so a binding written without them would stop matching the moment they
    /// were switched on. Both are removed here, and only here.
    pub const fn cleaned(self, numlock: ModMask) -> Self {
        let lock_bits = Modifier::CapsLock.bit();
        Self(self.0 & !(numlock.0 | lock_bits) & Self::ALL_BITS)
    }

    /// Every modifier position, for masking off anything that is not a modifier.
    const ALL_BITS: u16 = 0b1111_1111;

    /// This mask with every modifier in `other` cleared.
    ///
    /// [`std::ops::BitAndNot`] is still unstable, so this is the spelled-out
    /// form rather than a trait impl.
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// The modifiers held, in [`Modifier::DISPLAY_ORDER`].
    pub fn iter(self) -> impl Iterator<Item = Modifier> {
        let bits = self.0;
        Modifier::DISPLAY_ORDER
            .into_iter()
            .filter(move |modifier| bits & modifier.bit() != 0)
    }

    /// The modifiers held, named the way the i3bar click-event protocol names
    /// them. See [`Modifier::i3_name`].
    pub fn i3_names(self) -> impl Iterator<Item = &'static str> {
        let bits = self.0;
        Modifier::DISPLAY_ORDER
            .into_iter()
            .filter(move |modifier| bits & modifier.bit() != 0)
            .map(Modifier::i3_name)
    }

    /// A mask holding every modifier in `modifiers`.
    pub fn from_modifiers(modifiers: impl IntoIterator<Item = Modifier>) -> Self {
        modifiers
            .into_iter()
            .fold(Self::NONE, |mask, modifier| mask.with(modifier))
    }
}

impl BitOr for ModMask {
    type Output = ModMask;

    fn bitor(self, other: ModMask) -> ModMask {
        ModMask(self.0 | other.0)
    }
}

impl BitOrAssign for ModMask {
    fn bitor_assign(&mut self, other: ModMask) {
        self.0 |= other.0;
    }
}

impl BitAnd for ModMask {
    type Output = ModMask;

    fn bitand(self, other: ModMask) -> ModMask {
        ModMask(self.0 & other.0)
    }
}

impl fmt::Display for ModMask {
    /// Render as `Super + Control + Shift`, empty when no modifier is held.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rendered = self
            .iter()
            .map(|modifier| modifier.to_string())
            .collect::<Vec<_>>()
            .join(" + ");
        f.write_str(&rendered)
    }
}

impl FromStr for ModMask {
    type Err = ModifierNameError;

    /// Parse a chord's modifier list, such as `super+shift` or `Super + Shift`.
    ///
    /// `+`, spaces and commas all separate modifiers, and an empty string is a
    /// valid empty mask.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.split(['+', ',', ' ', '\t'])
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(Modifier::from_str)
            .collect::<Result<Vec<_>, _>>()
            .map(ModMask::from_modifiers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_names_round_trip_in_any_case() {
        for modifier in Modifier::DISPLAY_ORDER {
            let name = modifier.to_string();
            assert_eq!(name.parse::<Modifier>().unwrap(), modifier);
            assert_eq!(name.to_lowercase().parse::<Modifier>().unwrap(), modifier);
            assert_eq!(name.to_uppercase().parse::<Modifier>().unwrap(), modifier);
        }
    }

    #[test]
    fn aliases_are_rejected_so_the_language_has_one_spelling() {
        // `mod`/`modkey`/`mod4` for Super and `mod1` for Alt were accepted
        // before the modifier vocabulary was closed. They must not come back.
        for alias in [
            "mod", "modkey", "mod4", "mod1", "ctrl", "win", "cmd", "lock", "control_",
        ] {
            assert!(
                alias.parse::<Modifier>().is_err(),
                "'{alias}' must not resolve"
            );
        }
    }

    #[test]
    fn unknown_modifier_error_lists_the_valid_names() {
        let error = "hyper".parse::<Modifier>().unwrap_err().to_string();
        assert!(error.contains("unknown modifier 'hyper'"), "{error}");
        for modifier in Modifier::DISPLAY_ORDER {
            assert!(error.contains(&modifier.to_string()), "{error}");
        }
    }

    #[test]
    fn mask_renders_and_iterates_in_the_same_order() {
        let mask = ModMask::from_modifiers([Modifier::Shift, Modifier::Super, Modifier::Alt]);
        assert_eq!(mask.to_string(), "Super + Shift + Alt");
        assert_eq!(
            mask.iter().collect::<Vec<_>>(),
            vec![Modifier::Super, Modifier::Shift, Modifier::Alt]
        );
        assert!(mask.to_string().parse::<ModMask>().is_ok());
    }

    #[test]
    fn masks_accept_separators_and_an_empty_chord() {
        let expected = ModMask::from_modifiers([Modifier::Super, Modifier::Control]);
        for text in [
            "super+control",
            "Super + Control",
            "super, control",
            " super  control ",
        ] {
            assert_eq!(text.parse::<ModMask>().unwrap(), expected, "{text}");
        }
        assert_eq!("".parse::<ModMask>().unwrap(), ModMask::NONE);
        assert!(ModMask::NONE.is_empty());
    }

    #[test]
    fn every_modifier_round_trips_through_its_bit() {
        for modifier in Modifier::DISPLAY_ORDER {
            assert_eq!(Modifier::from_bit(modifier.bit()), Some(modifier));
            let mask = ModMask::from_modifier(modifier);
            assert!(mask.contains(modifier));
            assert_eq!(mask.without(modifier), ModMask::NONE);
            assert_eq!(mask.bits(), modifier.bit());
        }
    }

    #[test]
    fn caps_lock_and_numlock_are_dropped_when_cleaning() {
        let numlock = ModMask::from_modifier(Modifier::Mod2);
        let held = ModMask::from_modifiers([
            Modifier::Super,
            Modifier::CapsLock,
            Modifier::Mod2,
            Modifier::Shift,
        ]);
        let cleaned = held.cleaned(numlock);
        assert_eq!(
            cleaned,
            ModMask::from_modifiers([Modifier::Super, Modifier::Shift])
        );
    }

    #[test]
    fn i3_names_stay_positional_and_cover_caps_lock() {
        let mask = ModMask::from_modifiers([
            Modifier::Super,
            Modifier::CapsLock,
            Modifier::Control,
            Modifier::Alt,
        ]);
        // Positional, and in `DISPLAY_ORDER`, so Caps Lock trails. i3bar
        // scripts get "Lock" for Caps Lock, which the old hand-written table
        // omitted entirely.
        assert_eq!(
            mask.i3_names().collect::<Vec<_>>(),
            vec!["Mod4", "Control", "Mod1", "Lock"]
        );
    }

    #[test]
    fn bitwise_operators_compose_masks() {
        let super_key = ModMask::from_modifier(Modifier::Super);
        let shift = ModMask::from_modifier(Modifier::Shift);
        assert_eq!(
            super_key | shift,
            ModMask::from_modifiers([Modifier::Super, Modifier::Shift])
        );
        assert_eq!(
            (super_key | shift) & shift,
            shift,
            "intersection keeps shared modifiers"
        );
        assert_eq!(
            (super_key | shift).difference(shift),
            super_key,
            "difference drops the named modifier"
        );
    }
}
