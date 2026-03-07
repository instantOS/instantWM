use crate::bar::color::{rgba_from_hex, Rgba};

pub fn rgba_from_config(color: &str) -> Option<Rgba> {
    rgba_from_hex(color)
}
