pub type Rgba = [f32; 4];

pub fn rgba_from_color(color: &crate::drw::Color) -> Rgba {
    [
        color.color.color.red as f32 / 65535.0,
        color.color.color.green as f32 / 65535.0,
        color.color.color.blue as f32 / 65535.0,
        color.color.color.alpha as f32 / 65535.0,
    ]
}

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

pub fn rgba_from_hex_opt(color: Option<&str>) -> Option<Rgba> {
    rgba_from_hex(color?)
}
