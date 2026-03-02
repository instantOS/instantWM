use crate::bar::color::{rgba_from_hex, Rgba};
use crate::bar::paint::BarScheme;
use crate::config::{ColIndex, SchemeClose, SchemeHover, SchemeTag, SchemeWin};
use crate::globals::Globals;
use crate::types::{Client, Monitor};

pub fn rgba_from_config(color: &str) -> Option<Rgba> {
    rgba_from_hex(color)
}

fn scheme_from_strings(colors: &crate::types::ColorSchemeStrings) -> Option<BarScheme> {
    let fg = rgba_from_hex(&colors.fg)?;
    let bg = rgba_from_hex(&colors.bg)?;
    let detail = rgba_from_hex(&colors.detail)?;
    Some(BarScheme { fg, bg, detail })
}

pub fn status_scheme(g: &Globals) -> Option<BarScheme> {
    let fg = rgba_from_hex(g.cfg.statusbarcolors.get(ColIndex::Fg))?;
    let bg = rgba_from_hex(g.cfg.statusbarcolors.get(ColIndex::Bg))?;
    let detail = rgba_from_hex(g.cfg.statusbarcolors.get(ColIndex::Detail))?;
    Some(BarScheme { fg, bg, detail })
}

pub fn tag_scheme(
    g: &Globals,
    m: &Monitor,
    tag_index: u32,
    occupied_tags: u32,
    is_hover: bool,
) -> Option<BarScheme> {
    let scheme_idx = if occupied_tags & (1 << tag_index) != 0 {
        let sel_has_tag = g
            .selmon()
            .and_then(|selmon| {
                selmon.sel.and_then(|sel_win| {
                    g.clients
                        .get(&sel_win)
                        .map(|c| c.tags & (1 << tag_index) != 0)
                })
            })
            .unwrap_or(false);

        let is_selected = g.selmon().is_some_and(|selmon| selmon.num == m.num);

        if is_selected && sel_has_tag {
            SchemeTag::Focus
        } else if m.tagset[m.seltags as usize] & (1 << tag_index) != 0 {
            SchemeTag::NoFocus
        } else if m.showtags == 0 {
            SchemeTag::Filled
        } else {
            SchemeTag::Inactive
        }
    } else if m.tagset[m.seltags as usize] & (1 << tag_index) != 0 {
        SchemeTag::Empty
    } else {
        SchemeTag::Inactive
    };

    let colors = g.tags.colors.scheme(
        if is_hover {
            SchemeHover::Hover
        } else {
            SchemeHover::NoHover
        },
        scheme_idx,
    );
    scheme_from_strings(colors)
}

pub fn tag_hover_fill_scheme(g: &Globals) -> Option<BarScheme> {
    let colors = g.tags.colors.scheme(SchemeHover::Hover, SchemeTag::Filled);
    scheme_from_strings(colors)
}

pub fn window_scheme(g: &Globals, c: &Client, is_hover: bool) -> Option<BarScheme> {
    let is_selected = g.selmon().is_some_and(|selmon| selmon.sel == Some(c.win));
    let is_overlay = g
        .selmon()
        .is_some_and(|selmon| selmon.overlay == Some(c.win));

    let scheme_idx = if is_selected {
        if is_overlay {
            SchemeWin::OverlayFocus
        } else if c.issticky {
            SchemeWin::StickyFocus
        } else {
            SchemeWin::Focus
        }
    } else if is_overlay {
        SchemeWin::Overlay
    } else if c.issticky {
        SchemeWin::Sticky
    } else if c.is_hidden {
        SchemeWin::Minimized
    } else {
        SchemeWin::Normal
    };

    let colors = g.cfg.windowcolors.scheme(
        if is_hover {
            SchemeHover::Hover
        } else {
            SchemeHover::NoHover
        },
        scheme_idx,
    );
    scheme_from_strings(colors)
}

pub fn close_button_scheme(
    g: &Globals,
    is_hover: bool,
    is_locked: bool,
    is_fullscreen: bool,
) -> Option<BarScheme> {
    let scheme_idx = if is_locked {
        SchemeClose::Locked
    } else if is_fullscreen {
        SchemeClose::Fullscreen
    } else {
        SchemeClose::Normal
    };

    let colors = g.cfg.closebuttoncolors.scheme(
        if is_hover {
            SchemeHover::Hover
        } else {
            SchemeHover::NoHover
        },
        scheme_idx,
    );
    scheme_from_strings(colors)
}
