use crate::bar::color::{rgba_from_color, rgba_from_hex, rgba_from_hex_opt, Rgba};
use crate::bar::paint::BarScheme;
use crate::config::{SchemeClose, SchemeTag, SchemeWin};
use crate::globals::Globals;
use crate::types::{Client, Monitor};

pub fn rgba_from_config(color: &str) -> Option<Rgba> {
    rgba_from_hex(color)
}

pub fn status_scheme(g: &Globals) -> Option<BarScheme> {
    if let Some(ss) = g.cfg.statusscheme.as_ref() {
        return Some(BarScheme {
            fg: rgba_from_color(&ss.fg),
            bg: rgba_from_color(&ss.bg),
            detail: rgba_from_color(&ss.detail),
        });
    }
    let fg = rgba_from_hex_opt(g.cfg.statusbarcolors.first().copied());
    let bg = rgba_from_hex_opt(g.cfg.statusbarcolors.get(1).copied());
    let detail = rgba_from_hex_opt(g.cfg.statusbarcolors.get(2).copied());
    match (fg, bg, detail) {
        (Some(fg), Some(bg), Some(detail)) => Some(BarScheme { fg, bg, detail }),
        _ => None,
    }
}

pub fn tag_scheme(
    g: &Globals,
    m: &Monitor,
    tag_index: u32,
    occupied_tags: u32,
    is_hover: bool,
) -> Option<BarScheme> {
    let schemes = if is_hover {
        &g.tags.schemes.hover
    } else {
        &g.tags.schemes.no_hover
    };

    if schemes.is_empty() {
        return None;
    }

    let scheme = if occupied_tags & (1 << tag_index) != 0 {
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
            schemes.get(SchemeTag::Focus as usize)
        } else if m.tagset[m.seltags as usize] & (1 << tag_index) != 0 {
            schemes.get(SchemeTag::NoFocus as usize)
        } else if m.showtags == 0 {
            schemes.get(SchemeTag::Filled as usize)
        } else {
            schemes.get(SchemeTag::Inactive as usize)
        }
    } else if m.tagset[m.seltags as usize] & (1 << tag_index) != 0 {
        schemes.get(SchemeTag::Empty as usize)
    } else {
        schemes.get(SchemeTag::Inactive as usize)
    };

    scheme.map(|cs| BarScheme {
        fg: rgba_from_color(&cs.fg),
        bg: rgba_from_color(&cs.bg),
        detail: rgba_from_color(&cs.detail),
    })
}

pub fn tag_hover_fill_scheme(g: &Globals) -> Option<BarScheme> {
    g.tags
        .schemes
        .hover
        .get(SchemeTag::Filled as usize)
        .map(|cs| BarScheme {
            fg: rgba_from_color(&cs.fg),
            bg: rgba_from_color(&cs.bg),
            detail: rgba_from_color(&cs.detail),
        })
}

pub fn window_scheme(g: &Globals, c: &Client, is_hover: bool) -> Option<BarScheme> {
    let schemes = if is_hover {
        &g.cfg.windowschemes.hover
    } else {
        &g.cfg.windowschemes.no_hover
    };

    if schemes.is_empty() {
        return None;
    }

    let is_selected = g.selmon().is_some_and(|selmon| selmon.sel == Some(c.win));
    let is_overlay = g
        .selmon()
        .is_some_and(|selmon| selmon.overlay == Some(c.win));

    let scheme = if is_selected {
        if is_overlay {
            schemes.get(SchemeWin::OverlayFocus as usize)
        } else if c.issticky {
            schemes.get(SchemeWin::StickyFocus as usize)
        } else {
            schemes.get(SchemeWin::Focus as usize)
        }
    } else if is_overlay {
        schemes.get(SchemeWin::Overlay as usize)
    } else if c.issticky {
        schemes.get(SchemeWin::Sticky as usize)
    } else if c.is_hidden {
        schemes.get(SchemeWin::Minimized as usize)
    } else {
        schemes.get(SchemeWin::Normal as usize)
    };

    scheme.map(|cs| BarScheme {
        fg: rgba_from_color(&cs.fg),
        bg: rgba_from_color(&cs.bg),
        detail: rgba_from_color(&cs.detail),
    })
}

pub fn close_button_scheme(
    g: &Globals,
    is_hover: bool,
    is_locked: bool,
    is_fullscreen: bool,
) -> Option<BarScheme> {
    let schemes = if is_hover {
        &g.cfg.closebuttonschemes.hover
    } else {
        &g.cfg.closebuttonschemes.no_hover
    };

    if schemes.is_empty() {
        return None;
    }

    let scheme_idx = if is_locked {
        SchemeClose::Locked as usize
    } else if is_fullscreen {
        SchemeClose::Fullscreen as usize
    } else {
        SchemeClose::Normal as usize
    };

    schemes.get(scheme_idx).map(|cs| BarScheme {
        fg: rgba_from_color(&cs.fg),
        bg: rgba_from_color(&cs.bg),
        detail: rgba_from_color(&cs.detail),
    })
}
