use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An RGBA color represented by floating-point components.
///
/// Components are expected to be in `[0.0, 1.0]`; byte conversions clamp
/// values outside that range.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgba([f32; 4]);

impl Rgba {
    pub const ZERO: Self = Self([0.0; 4]);

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self([r, g, b, a])
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self([r, g, b, 1.0])
    }

    pub const fn r(self) -> f32 {
        self.0[0]
    }

    pub const fn g(self) -> f32 {
        self.0[1]
    }

    pub const fn b(self) -> f32 {
        self.0[2]
    }

    pub const fn a(self) -> f32 {
        self.0[3]
    }

    pub const fn with_alpha(self, alpha: f32) -> Self {
        Self::new(self.r(), self.g(), self.b(), alpha)
    }

    pub const fn into_array(self) -> [f32; 4] {
        self.0
    }

    /// Convert the components to bytes, clamping them to the normalized range
    /// and rounding to the nearest integer.
    pub fn to_rgba8(self) -> [u8; 4] {
        fn component_to_u8(component: f32) -> u8 {
            (component.clamp(0.0, 1.0) * 255.0).round() as u8
        }

        [
            component_to_u8(self.r()),
            component_to_u8(self.g()),
            component_to_u8(self.b()),
            component_to_u8(self.a()),
        ]
    }
}

impl std::fmt::Display for Rgba {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [r, g, b, a] = self.to_rgba8();
        if a == 255 {
            write!(f, "#{:02X}{:02X}{:02X}", r, g, b)
        } else {
            write!(f, "#{:02X}{:02X}{:02X}{:02X}", r, g, b, a)
        }
    }
}

/// Pack the color as `0xRRGGBB`. The alpha component is discarded.
impl From<Rgba> for u32 {
    fn from(rgba: Rgba) -> Self {
        let [r, g, b, _] = rgba.to_rgba8();
        let (r, g, b) = (u32::from(r), u32::from(g), u32::from(b));
        (r << 16) | (g << 8) | b
    }
}

impl std::str::FromStr for Rgba {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let hex = s.strip_prefix('#').unwrap_or(s);
        if hex.len() != 6 && hex.len() != 8 {
            return Err(format!("invalid hex color: {s}"));
        }
        let parse = |range: std::ops::Range<usize>| -> Result<u8, String> {
            u8::from_str_radix(&hex[range], 16).map_err(|_| format!("invalid hex color: {s}"))
        };
        let r = parse(0..2)?;
        let g = parse(2..4)?;
        let b = parse(4..6)?;
        let a = if hex.len() == 8 { parse(6..8)? } else { 255 };
        Ok(Self([
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        ]))
    }
}

impl Serialize for Rgba {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let rgba = Rgba::new(0.5, 0.25, 0.75, 1.0);
        let hex = rgba.to_string();
        assert_eq!(hex, "#8040BF");
        let parsed = hex.parse::<Rgba>().unwrap();
        // Hex roundtrip isn't exact for floats (0.5*255=127.5→128, 128/255≠0.5)
        // but the hex string roundtrips exactly.
        assert_eq!(parsed.to_string(), hex);
    }

    #[test]
    fn hex_with_alpha() {
        let rgba = Rgba::new(1.0, 0.0, 0.0, 0.5);
        let hex = rgba.to_string();
        assert_eq!(hex, "#FF000080");
        let parsed: Rgba = hex.parse().unwrap();
        assert_eq!(parsed.to_string(), hex);
    }

    #[test]
    fn to_u32() {
        let rgba = Rgba::new(1.0, 0.5, 0.0, 1.0);
        let packed: u32 = rgba.into();
        assert_eq!(packed, 0xFF8000);
    }

    #[test]
    fn rgba8_is_clamped_and_rounded() {
        let rgba = Rgba::new(-0.1, 0.5, 1.1, 0.25);
        assert_eq!(rgba.to_rgba8(), [0, 128, 255, 64]);
    }

    #[test]
    fn debug_preserves_precision_hidden_by_display() {
        let first = Rgba::rgb(0.5, 0.0, 0.0);
        let second = Rgba::rgb(0.5001, 0.0, 0.0);
        assert_eq!(first.to_string(), second.to_string());
        assert_ne!(format!("{first:?}"), format!("{second:?}"));
    }
}
