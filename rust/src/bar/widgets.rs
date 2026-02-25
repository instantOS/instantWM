use crate::config::{SchemeClose, SchemeTag, SchemeWin};
use crate::contexts::WmCtx;
use crate::drw::{Drw, COL_BG, COL_DETAIL};
use crate::globals::{get_drw, get_drw_mut, get_globals, Globals};
use crate::types::*;

const DETAIL_BAR_HEIGHT_NORMAL: i32 = 4;
const DETAIL_BAR_HEIGHT_HOVER: i32 = 8;
const STARTMENU_ICON_SIZE: i32 = 14;
const STARTMENU_ICON_INNER: i32 = 6;

pub(crate) fn draw_startmenu_icon(bh: i32) {
    let g = get_globals();
    let icon_offset = (bh - CLOSE_BUTTON_WIDTH) / 2;
    let startmenu_invert = g
        .monitors
        .get(g.selmon)
        .is_some_and(|mon| mon.gesture == Gesture::StartMenu);

    let startmenu_size = g.cfg.startmenusize;
    let scheme: Option<ColorScheme> = if g.tags.prefix {
        let schemes = &g.tags.schemes;
        if !schemes.no_hover.is_empty() {
            schemes.no_hover.get(SchemeTag::Focus as usize).cloned()
        } else {
            g.cfg.statusscheme.as_ref().map(|s| ColorScheme {
                fg: s.fg.clone(),
                bg: s.bg.clone(),
                detail: s.detail.clone(),
            })
        }
    } else {
        g.cfg.statusscheme.as_ref().map(|s| ColorScheme {
            fg: s.fg.clone(),
            bg: s.bg.clone(),
            detail: s.detail.clone(),
        })
    };

    let Some(ref scheme) = scheme else { return };

    let mut drw = get_drw().clone();
    drw.set_scheme(scheme.clone());

    drw.rect(
        0,
        0,
        startmenu_size as u32,
        bh as u32,
        true,
        !startmenu_invert,
    );
    drw.rect(
        5,
        icon_offset,
        STARTMENU_ICON_SIZE as u32,
        STARTMENU_ICON_SIZE as u32,
        true,
        startmenu_invert,
    );
    drw.rect(
        9,
        icon_offset + 4,
        STARTMENU_ICON_INNER as u32,
        STARTMENU_ICON_INNER as u32,
        true,
        !startmenu_invert,
    );
    drw.rect(
        19,
        icon_offset + STARTMENU_ICON_SIZE,
        STARTMENU_ICON_INNER as u32,
        STARTMENU_ICON_INNER as u32,
        true,
        startmenu_invert,
    );
}

/// Draw start menu icon with dependency injection
pub(crate) fn draw_startmenu_icon_ctx(ctx: &mut WmCtx) {
    let g = &*ctx.g;
    let bh = ctx.g.cfg.bh;
    let icon_offset = (bh - CLOSE_BUTTON_WIDTH) / 2;

    // Check if start menu is hovered by looking at the selected monitor's gesture
    let startmenu_invert = g
        .monitors
        .get(g.selmon)
        .is_some_and(|mon| mon.gesture == Gesture::StartMenu);

    let startmenu_size = g.cfg.startmenusize;
    let scheme: Option<ColorScheme> = if g.tags.prefix {
        let schemes = &g.tags.schemes;
        if !schemes.no_hover.is_empty() {
            schemes.no_hover.get(SchemeTag::Focus as usize).cloned()
        } else {
            g.cfg.statusscheme.as_ref().map(|s| ColorScheme {
                fg: s.fg.clone(),
                bg: s.bg.clone(),
                detail: s.detail.clone(),
            })
        }
    } else {
        g.cfg.statusscheme.as_ref().map(|s| ColorScheme {
            fg: s.fg.clone(),
            bg: s.bg.clone(),
            detail: s.detail.clone(),
        })
    };

    let Some(ref scheme) = scheme else { return };

    let drw = get_drw_mut();
    drw.set_scheme(scheme.clone());

    drw.rect(
        0,
        0,
        startmenu_size as u32,
        bh as u32,
        true,
        !startmenu_invert,
    );
    drw.rect(
        5,
        icon_offset,
        STARTMENU_ICON_SIZE as u32,
        STARTMENU_ICON_SIZE as u32,
        true,
        startmenu_invert,
    );
    drw.rect(
        9,
        icon_offset + 4,
        STARTMENU_ICON_INNER as u32,
        STARTMENU_ICON_INNER as u32,
        true,
        !startmenu_invert,
    );
    drw.rect(
        19,
        icon_offset + STARTMENU_ICON_SIZE,
        STARTMENU_ICON_INNER as u32,
        STARTMENU_ICON_INNER as u32,
        true,
        startmenu_invert,
    );
}

fn get_tag_scheme_from_globals(
    g: &Globals,
    m: &Monitor,
    i: u32,
    occupied_tags: u32,
    is_hover: bool,
) -> Option<ColorScheme> {
    let schemes = if is_hover {
        &g.tags.schemes.hover
    } else {
        &g.tags.schemes.no_hover
    };

    if schemes.is_empty() {
        return None;
    }

    if occupied_tags & (1 << i) != 0 {
        let sel_has_tag = g
            .monitors
            .get(g.selmon)
            .and_then(|selmon| {
                selmon
                    .sel
                    .and_then(|sel_win| g.clients.get(&sel_win).map(|c| c.tags & (1 << i) != 0))
            })
            .unwrap_or(false);

        let is_selected = g
            .monitors
            .get(g.selmon)
            .is_some_and(|selmon| selmon.num == m.num);

        if is_selected && sel_has_tag {
            return schemes.get(SchemeTag::Focus as usize).cloned();
        }
        if m.tagset[m.seltags as usize] & (1 << i) != 0 {
            return schemes.get(SchemeTag::NoFocus as usize).cloned();
        }
        if m.showtags == 0 {
            return schemes.get(SchemeTag::Filled as usize).cloned();
        }
        return schemes.get(SchemeTag::Inactive as usize).cloned();
    }

    if m.tagset[m.seltags as usize] & (1 << i) != 0 {
        return schemes.get(SchemeTag::Empty as usize).cloned();
    }
    schemes.get(SchemeTag::Inactive as usize).cloned()
}

fn get_tag_scheme(m: &Monitor, i: u32, occupied_tags: u32, is_hover: bool) -> Option<ColorScheme> {
    let g = get_globals();
    get_tag_scheme_from_globals(g, m, i, occupied_tags, is_hover)
}

/// Get tag scheme with dependency injection
fn get_tag_scheme_ctx(
    ctx: &WmCtx,
    m: &Monitor,
    i: u32,
    occupied_tags: u32,
    is_hover: bool,
) -> Option<ColorScheme> {
    get_tag_scheme_from_globals(ctx.g, m, i, occupied_tags, is_hover)
}

pub(crate) fn draw_tag_indicators(
    m: &mut Monitor,
    mut x: i32,
    occupied_tags: u32,
    urg: u32,
    bh: i32,
) -> i32 {
    let g = get_globals();
    let lrpad = g.cfg.lrpad;
    let lpad = (lrpad / 2) as u32;
    let bar_dragging = g.bar_dragging;

    let tags = crate::tags::bar::visible_tags(g, m, occupied_tags);

    let mut drw = get_drw().clone();

    let selmon_gesture = g
        .monitors
        .get(g.selmon)
        .map(|s| s.gesture)
        .unwrap_or_default();

    for t in &tags {
        // A tag cell is hovered when the current gesture is Tag(slot) for this cell's slot.
        let is_hover = selmon_gesture == Gesture::Tag(t.slot);

        let Some(scheme) = get_tag_scheme(m, t.tag_index as u32, occupied_tags, is_hover) else {
            x += t.width;
            continue;
        };

        let mut draw_scheme = scheme;
        if is_hover && bar_dragging {
            if let Some(s) = g.tags.schemes.hover.get(SchemeTag::Filled as usize) {
                draw_scheme = s.clone();
            }
        }
        drw.set_scheme(draw_scheme);

        let detail_height = if is_hover {
            DETAIL_BAR_HEIGHT_HOVER
        } else {
            DETAIL_BAR_HEIGHT_NORMAL
        };

        x = drw.text(
            x,
            0,
            t.width as u32,
            bh as u32,
            lpad,
            t.label,
            urg & (1 << t.tag_index) != 0,
            detail_height,
        );
    }

    x
}

/// Draw tag indicators with dependency injection
pub(crate) fn draw_tag_indicators_ctx(
    ctx: &mut WmCtx,
    m: &Monitor,
    mut x: i32,
    occupied_tags: u32,
    urg: u32,
) -> i32 {
    let g = &*ctx.g;
    let bh = ctx.g.cfg.bh;
    let lrpad = ctx.g.cfg.lrpad;
    let lpad = (lrpad / 2) as u32;
    let bar_dragging = g.bar_dragging;

    let tags = crate::tags::bar::visible_tags(g, m, occupied_tags);

    let selmon_gesture = g
        .monitors
        .get(g.selmon)
        .map(|s| s.gesture)
        .unwrap_or_default();

    for t in &tags {
        // A tag cell is hovered when the current gesture is Tag(slot) for this cell's slot.
        let is_hover = selmon_gesture == Gesture::Tag(t.slot);

        let Some(scheme) = get_tag_scheme_ctx(ctx, m, t.tag_index as u32, occupied_tags, is_hover)
        else {
            x += t.width;
            continue;
        };

        let mut draw_scheme = scheme;
        if is_hover && bar_dragging {
            if let Some(s) = g.tags.schemes.hover.get(SchemeTag::Filled as usize) {
                draw_scheme = s.clone();
            }
        }
        let drw = get_drw_mut();
        drw.set_scheme(draw_scheme);

        let detail_height = if is_hover {
            DETAIL_BAR_HEIGHT_HOVER
        } else {
            DETAIL_BAR_HEIGHT_NORMAL
        };

        x = drw.text(
            x,
            0,
            t.width as u32,
            bh as u32,
            lpad,
            t.label,
            urg & (1 << t.tag_index) != 0,
            detail_height,
        );
    }

    x
}

pub(crate) fn draw_layout_indicator(m: &Monitor, mut x: i32, bh: i32) -> i32 {
    let g = get_globals();
    let lrpad = g.cfg.lrpad;
    let ltsymbol = super::layout_symbol(m);
    let text_w = super::text_width(&ltsymbol);
    let w = (text_w + lrpad).max(lrpad);
    let lpad = ((w - text_w) / 2).max(0) as u32;

    {
        let mut drw = get_drw().clone();
        if let Some(ref ss) = g.cfg.statusscheme {
            let scheme = ColorScheme {
                fg: ss.fg.clone(),
                bg: ss.bg.clone(),
                detail: ss.detail.clone(),
            };
            drw.set_scheme(scheme);
        }
        x = drw.text(x, 0, w as u32, bh as u32, lpad, &ltsymbol, false, 0);
    }

    x
}

/// Draw layout indicator with dependency injection
pub(crate) fn draw_layout_indicator_ctx(ctx: &mut WmCtx, m: &Monitor, mut x: i32) -> i32 {
    let g = &*ctx.g;
    let lrpad = ctx.g.cfg.lrpad;
    let ltsymbol = super::layout_symbol(m);
    let text_w = super::text_width(&ltsymbol);
    let w = (text_w + lrpad).max(lrpad);
    let lpad = ((w - text_w) / 2).max(0) as u32;
    let bh = ctx.g.cfg.bh;

    if let Some(ref ss) = g.cfg.statusscheme {
        let scheme = ColorScheme {
            fg: ss.fg.clone(),
            bg: ss.bg.clone(),
            detail: ss.detail.clone(),
        };
        let drw = get_drw_mut();
        drw.set_scheme(scheme);
    }
    x = get_drw_mut().text(x, 0, w as u32, bh as u32, lpad, &ltsymbol, false, 0);

    x
}

fn get_window_scheme(g: &Globals, c: &Client, is_hover: bool) -> Option<ColorScheme> {
    let schemes = if is_hover {
        &g.cfg.windowschemes.hover
    } else {
        &g.cfg.windowschemes.no_hover
    };

    if schemes.is_empty() {
        return None;
    }

    let is_selected = g
        .monitors
        .get(g.selmon)
        .is_some_and(|selmon| selmon.sel == Some(c.win));

    let is_overlay = g
        .monitors
        .get(g.selmon)
        .is_some_and(|selmon| selmon.overlay == Some(c.win));

    if is_selected {
        if is_overlay {
            return schemes.get(SchemeWin::OverlayFocus as usize).cloned();
        }
        if c.issticky {
            return schemes.get(SchemeWin::StickyFocus as usize).cloned();
        }
        return schemes.get(SchemeWin::Focus as usize).cloned();
    }

    if is_overlay {
        return schemes.get(SchemeWin::Overlay as usize).cloned();
    }
    if c.issticky {
        return schemes.get(SchemeWin::Sticky as usize).cloned();
    }
    if c.is_hidden {
        return schemes.get(SchemeWin::Minimized as usize).cloned();
    }
    schemes.get(SchemeWin::Normal as usize).cloned()
}

pub(crate) fn draw_close_button(c: &Client, x: i32, bh: i32) {
    let g = get_globals();

    let close_hovered = g
        .monitors
        .get(g.selmon)
        .is_some_and(|selmon| selmon.gesture == Gesture::CloseButton);

    let schemes = if close_hovered {
        &g.cfg.closebuttonschemes.hover
    } else {
        &g.cfg.closebuttonschemes.no_hover
    };

    {
        let mut drw = get_drw().clone();

        let scheme_idx = if c.islocked {
            SchemeClose::Locked as usize
        } else if g
            .monitors
            .get(g.selmon)
            .and_then(|selmon| {
                selmon.sel.and_then(|sel_win| {
                    g.clients
                        .get(&sel_win)
                        .map(|sel_c| sel_c.is_fullscreen && sel_c.win == c.win)
                })
            })
            .unwrap_or(false)
        {
            SchemeClose::Fullscreen as usize
        } else {
            SchemeClose::Normal as usize
        };

        if let Some(scheme) = schemes.get(scheme_idx) {
            drw.set_scheme(scheme.clone());
        }

        let button_x = x + bh / 6;
        let detail_offset = if close_hovered {
            CLOSE_BUTTON_DETAIL
        } else {
            0
        };
        let button_y = (bh - CLOSE_BUTTON_WIDTH) / 2 - detail_offset;

        unsafe {
            crate::drw::XSetForeground(drw.display(), drw.gc(), get_scheme_pixel(&drw, COL_BG));
            crate::drw::XFillRectangle(
                drw.display(),
                drw.drawable(),
                drw.gc(),
                button_x,
                button_y,
                CLOSE_BUTTON_WIDTH as u32,
                CLOSE_BUTTON_HEIGHT as u32,
            );
            crate::drw::XSetForeground(drw.display(), drw.gc(), get_scheme_pixel(&drw, COL_DETAIL));
            crate::drw::XFillRectangle(
                drw.display(),
                drw.drawable(),
                drw.gc(),
                button_x,
                (bh - CLOSE_BUTTON_WIDTH) / 2 + CLOSE_BUTTON_HEIGHT - detail_offset,
                CLOSE_BUTTON_WIDTH as u32,
                (CLOSE_BUTTON_DETAIL + detail_offset) as u32,
            );
        }
    }
}

/// Draw close button with dependency injection
pub(crate) fn draw_close_button_ctx(c: &Client, ctx: &mut WmCtx, x: i32) {
    let g = &*ctx.g;
    let bh = ctx.g.cfg.bh;
    let close_hovered = g
        .monitors
        .get(g.selmon)
        .is_some_and(|selmon| selmon.gesture == Gesture::CloseButton);

    let schemes = if close_hovered {
        &g.cfg.closebuttonschemes.hover
    } else {
        &g.cfg.closebuttonschemes.no_hover
    };

    let scheme_idx = if c.islocked {
        SchemeClose::Locked as usize
    } else if g
        .monitors
        .get(g.selmon)
        .and_then(|selmon| {
            selmon.sel.and_then(|sel_win| {
                g.clients
                    .get(&sel_win)
                    .map(|sel_c| sel_c.is_fullscreen && sel_c.win == c.win)
            })
        })
        .unwrap_or(false)
    {
        SchemeClose::Fullscreen as usize
    } else {
        SchemeClose::Normal as usize
    };

    if let Some(scheme) = schemes.get(scheme_idx) {
        let drw = get_drw_mut();
        drw.set_scheme(scheme.clone());
    }

    let button_x = x + bh / 6;
    let detail_offset = if close_hovered {
        CLOSE_BUTTON_DETAIL
    } else {
        0
    };
    let button_y = (bh - CLOSE_BUTTON_WIDTH) / 2 - detail_offset;

    let drw = get_drw_mut();
    unsafe {
        crate::drw::XSetForeground(drw.display(), drw.gc(), get_scheme_pixel(&drw, COL_BG));
        crate::drw::XFillRectangle(
            drw.display(),
            drw.drawable(),
            drw.gc(),
            button_x,
            button_y,
            CLOSE_BUTTON_WIDTH as u32,
            CLOSE_BUTTON_HEIGHT as u32,
        );
        crate::drw::XSetForeground(drw.display(), drw.gc(), get_scheme_pixel(&drw, COL_DETAIL));
        crate::drw::XFillRectangle(
            drw.display(),
            drw.drawable(),
            drw.gc(),
            button_x,
            (bh - CLOSE_BUTTON_WIDTH) / 2 + CLOSE_BUTTON_HEIGHT - detail_offset,
            CLOSE_BUTTON_WIDTH as u32,
            (CLOSE_BUTTON_DETAIL + detail_offset) as u32,
        );
    }
}

fn get_scheme_pixel(drw: &Drw, idx: usize) -> std::os::raw::c_ulong {
    if let Some(scheme) = drw.get_scheme() {
        if scheme.len() > idx {
            return scheme[idx].pixel() as std::os::raw::c_ulong;
        }
    }
    0
}

fn draw_window_title(m: &mut Monitor, c: &Client, x: i32, width: i32, bh: i32) {
    let g = get_globals();

    let is_hover = g.monitors.get(g.selmon).is_some_and(|selmon| {
        selmon.gesture == Gesture::None
            && selmon.sel.is_some_and(|sel_win| {
                g.clients
                    .get(&sel_win)
                    .is_some_and(|hover_c| hover_c.win == c.win)
            })
    });

    let client_name = c.name.as_str();
    let text_w = super::text_width(client_name);

    {
        let mut drw = get_drw().clone();
        if let Some(scheme) = get_window_scheme(g, c, is_hover) {
            drw.set_scheme(scheme);
        }

        let lpad = if text_w < width - 64 {
            ((width - text_w) as f32 * 0.5) as u32
        } else {
            (g.cfg.lrpad / 2 + 20) as u32
        };

        drw.text(x, 0, width as u32, bh as u32, lpad, client_name, false, 4);
    }

    let is_selected = g
        .monitors
        .get(g.selmon)
        .is_some_and(|selmon| selmon.sel == Some(c.win));

    if is_selected {
        draw_close_button(c, x, bh);
        m.activeoffset = m.monitor_rect.x as u32 + x as u32;
    }
}

/// Draw window title with dependency injection
fn draw_window_title_ctx(m: &mut Monitor, c: &Client, ctx: &mut WmCtx, x: i32, width: i32) {
    let g = &*ctx.g;
    let is_hover = g.monitors.get(g.selmon).is_some_and(|selmon| {
        selmon.gesture == Gesture::None
            && selmon.sel.is_some_and(|sel_win| {
                g.clients
                    .get(&sel_win)
                    .is_some_and(|hover_c| hover_c.win == c.win)
            })
    });

    let client_name = c.name.as_str();
    let text_w = super::text_width(client_name);
    let bh = ctx.g.cfg.bh;

    {
        if let Some(scheme) = get_window_scheme(g, c, is_hover) {
            let drw = get_drw_mut();
            drw.set_scheme(scheme);
        }

        let lpad = if text_w < width - 64 {
            ((width - text_w) as f32 * 0.5) as u32
        } else {
            (g.cfg.lrpad / 2 + 20) as u32
        };

        get_drw_mut().text(x, 0, width as u32, bh as u32, lpad, client_name, false, 4);
    }

    let is_selected = g
        .monitors
        .get(g.selmon)
        .is_some_and(|selmon| selmon.sel == Some(c.win));

    if is_selected {
        draw_close_button_ctx(c, ctx, x);
        m.activeoffset = m.monitor_rect.x as u32 + x as u32;
    }
}

pub(crate) fn draw_window_titles(m: &mut Monitor, x: i32, w: i32, n: i32, bh: i32) {
    let g = get_globals();

    if n > 0 {
        let total_width = w + 1;
        let each_width = total_width / n;
        let mut remainder = total_width % n;
        let mut x = x;

        // Walk the intrusive linked list so the draw order matches the order
        // used by bar_position_at_x — HashMap iteration order is non-deterministic
        // and would cause click regions to map to the wrong window titles.
        // Use the passed monitor `m` (not selmon) so that secondary monitors
        // draw their own clients, not the selected monitor's clients.
        let mut current = m.clients;
        while let Some(c_win) = current {
            let Some(c) = g.clients.get(&c_win) else {
                break;
            };
            current = c.next;

            if !c.is_visible() {
                continue;
            }

            let c = c.clone();
            let this_width = if remainder > 0 {
                remainder -= 1;
                each_width + 1
            } else {
                each_width
            };

            draw_window_title(m, &c, x, this_width, bh);
            x += this_width;
        }
        return;
    }

    {
        let mut drw = get_drw().clone();
        if let Some(ref ss) = g.cfg.statusscheme {
            let scheme = ColorScheme {
                fg: ss.fg.clone(),
                bg: ss.bg.clone(),
                detail: ss.detail.clone(),
            };
            drw.set_scheme(scheme);
        }
        drw.rect(x, 0, w as u32, bh as u32, true, true);

        let has_clients = g
            .monitors
            .get(g.selmon)
            .is_some_and(|selmon| selmon.clients.is_some());

        if !has_clients {
            let help_text = "Press space to launch an application";
            let text_w = super::text_width(help_text);
            let avail = w - bh;
            let title_width = text_w.min(avail);
            drw.text(
                x + bh + ((avail - title_width + 1) / 2),
                0,
                title_width as u32,
                bh as u32,
                0,
                help_text,
                false,
                0,
            );
        }
    }
}

/// Draw window titles with dependency injection
pub(crate) fn draw_window_titles_ctx(m: &mut Monitor, ctx: &mut WmCtx, x: i32, w: i32, n: i32) {
    let g = &*ctx.g;
    if n > 0 {
        let total_width = w + 1;
        let each_width = total_width / n;
        let mut remainder = total_width % n;
        let mut x = x;

        // Walk the intrusive linked list so the draw order matches the order
        // used by bar_position_at_x — HashMap iteration order is non-deterministic
        // and would cause click regions to map to the wrong window titles.
        // Use the passed monitor `m` (not selmon) so that secondary monitors
        // draw their own clients, not the selected monitor's clients.
        let mut current = m.clients;
        while let Some(c_win) = current {
            let Some(c) = g.clients.get(&c_win) else {
                break;
            };
            current = c.next;

            if !c.is_visible() {
                continue;
            }

            let c = c.clone();
            let this_width = if remainder > 0 {
                remainder -= 1;
                each_width + 1
            } else {
                each_width
            };

            draw_window_title_ctx(m, &c, ctx, x, this_width);
            x += this_width;
        }
        return;
    }

    if let Some(ref ss) = g.cfg.statusscheme {
        let scheme = ColorScheme {
            fg: ss.fg.clone(),
            bg: ss.bg.clone(),
            detail: ss.detail.clone(),
        };
        let drw = get_drw_mut();
        drw.set_scheme(scheme);
    }
    let bh = ctx.g.cfg.bh;
    get_drw_mut().rect(x, 0, w as u32, bh as u32, true, true);

    let has_clients = g
        .monitors
        .get(g.selmon)
        .is_some_and(|selmon| selmon.clients.is_some());

    if !has_clients {
        let help_text = "Press space to launch an application";
        let text_w = super::text_width(help_text);
        let avail = w - bh;
        let title_width = text_w.min(avail);
        get_drw_mut().text(
            x + bh + ((avail - title_width + 1) / 2),
            0,
            title_width as u32,
            bh as u32,
            0,
            help_text,
            false,
            0,
        );
    }
}
