use crate::bar::color::{Rgba, rgba_from_hex};

pub fn rgba_from_config(color: &str) -> Option<Rgba> {
    rgba_from_hex(color)
}
