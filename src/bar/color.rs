pub type Rgba = [f32; 4];

/// Parse a hex color string (e.g., "#RRGGBB" or "RRGGBB") to RGBA.
pub fn rgba_from_hex(color: &str) -> Option<Rgba> {
    let s = color.trim();
    let hex = s.strip_prefix('#').unwrap_or(s);
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    let a = if hex.len() == 8 {
        u8::from_str_radix(&hex[6..8], 16).ok()?
    } else {
        255
    };
    Some([
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ])
}

/// Convert RGBA to a hex color string.
pub fn rgba_to_hex(rgba: Rgba) -> String {
    let r = (rgba[0] * 255.0).round() as u8;
    let g = (rgba[1] * 255.0).round() as u8;
    let b = (rgba[2] * 255.0).round() as u8;
    let a = (rgba[3] * 255.0).round() as u8;
    if a == 255 {
        format!("#{:02X}{:02X}{:02X}", r, g, b)
    } else {
        format!("#{:02X}{:02X}{:02X}{:02X}", r, g, b, a)
    }
}

/// Convert RGBA to a packed u32 (0xRRGGBB).
pub fn rgba_to_u32(rgba: Rgba) -> u32 {
    let r = (rgba[0] * 255.0).round() as u32;
    let g = (rgba[1] * 255.0).round() as u32;
    let b = (rgba[2] * 255.0).round() as u32;
    (r << 16) | (g << 8) | b
}

/// Deserialize a hex color string to RGBA (for serde).
pub fn deserialize_hex_color<'de, D>(deserializer: D) -> Result<Rgba, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let s = String::deserialize(deserializer)?;
    rgba_from_hex(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid hex color: {}", s)))
}

/// Serialize an RGBA color to a hex string (for serde).
pub fn serialize_hex_color<S>(rgba: &Rgba, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&rgba_to_hex(*rgba))
}

/// Parse a hex color string (e.g., "#RRGGBB" or "RRGGBB") to a packed u32.
/// Returns the default color (0x121212) if parsing fails.
pub fn hex_to_u32(color: &str) -> u32 {
    let color = color.trim_start_matches('#');
    if color.len() == 6 {
        let r = u32::from_str_radix(&color[0..2], 16).unwrap_or(0);
        let g = u32::from_str_radix(&color[2..4], 16).unwrap_or(0);
        let b = u32::from_str_radix(&color[4..6], 16).unwrap_or(0);
        (r << 16) | (g << 8) | b
    } else {
        0x121212
    }
}
