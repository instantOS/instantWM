use crate::bar::color::{rgba_from_hex, Rgba};
use crate::bar::paint::BarScheme;
use crate::config::{ColIndex, SchemeClose, SchemeHover, SchemeTag, SchemeWin};
use crate::globals::Globals;
use crate::types::{Client, Monitor};

pub fn rgba_from_config(color: &str) -> Option<Rgba> {
    rgba_from_hex(color)
}

pub(crate) fn scheme_from_strings(colors: &crate::types::ColorSchemeStrings) -> Option<BarScheme> {
    let fg = rgba_from_hex(&colors.fg)?;
    let bg = rgba_from_hex(&colors.bg)?;
    let detail = rgba_from_hex(&colors.detail)?;
    Some(BarScheme { fg, bg, detail })
}
