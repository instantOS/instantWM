use crate::config::{SchemeClose, SchemeTag, SchemeWin};
use crate::contexts::WmCtx;
use crate::drw::{Drw, COL_BG, COL_DETAIL};
use crate::globals::{get_drw, get_globals, Globals};
use crate::types::*;

const DETAIL_BAR_HEIGHT_NORMAL: i32 = 4;
const DETAIL_BAR_HEIGHT_HOVER: i32 = 8;
const STARTMENU_ICON_SIZE: i32 = 14;
const STARTMENU_ICON_INNER: i32 = 6;

pub(crate) fn draw_startmenu_icon(ctx: &WmCtx, bh: i32) {
    let icon_offset = (bh - CLOSE_BUTTON_WIDTH) / 2;
    let startmenu_invert = ctx
        .g
        .monitors
        .get(ctx.g.selmon)
        .is_some_and(|mon| mon.gesture == Gesture::StartMenu);

    let startmenu_size = ctx.g.cfg.startmenusize;
    let scheme: Option<ColorScheme> = if ctx.g.tags.prefix {
        let schemes = &ctx.g.tags.schemes;
        if !schemes.no_hover.is_empty() {
            schemes.no_hover.get(SchemeTag::Focus as usize).cloned()
        } else {
            ctx.g.cfg.statusscheme.as_ref().map(|s| ColorScheme {
                fg: s.fg.clone(),
                bg: s.bg.clone(),
                detail: s.detail.clone(),
            })
        }
    } else {
        ctx.g.cfg.statusscheme.as_ref().map(|s| ColorScheme {
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

fn get_tag_scheme(
    ctx: &WmCtx,
    m: &Monitor,
    i: u32,
    occupied_tags: u32,
    is_hover: bool,
) -> Option<ColorScheme> {
    get_tag_scheme_from_globals(ctx.g, m, i, occupied_tags, is_hover)
}

pub(crate) fn draw_tag_indicators(
    ctx: &WmCtx,
    m: &Monitor,
    mut x: i32,
    occupied_tags: u32,
    urg: u32,
    bh: i32,
) -> i32 {
    let lrpad = ctx.g.cfg.lrpad;
    let lpad = (lrpad / 2) as u32;
    let bar_dragging = ctx.g.bar_dragging;

    let tags = crate::tags::bar::visible_tags(ctx.g, m, occupied_tags);

    let mut drw = get_drw().clone();

    let selmon_gesture = ctx
        .g
        .monitors
        .get(ctx.g.selmon)
        .map(|s| s.gesture)
        .unwrap_or_default();

    for t in &tags {
        // A tag cell is hovered when the current gesture is Tag(slot) for this cell's slot.
        let is_hover = selmon_gesture == Gesture::Tag(t.slot);

        let Some(scheme) = get_tag_scheme(ctx, m, t.tag_index as u32, occupied_tags, is_hover)
        else {
            x += t.width;
            continue;
        };

        let mut draw_scheme = scheme;
        if is_hover && bar_dragging {
            if let Some(s) = ctx.g.tags.schemes.hover.get(SchemeTag::Filled as usize) {
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

pub(crate) fn draw_layout_indicator(ctx: &WmCtx, m: &Monitor, mut x: i32, bh: i32) -> i32 {
    let lrpad = ctx.g.cfg.lrpad;
    let ltsymbol = super::layout_symbol(m);
    let text_w = super::text_width(&ltsymbol);
    let w = (text_w + lrpad).max(lrpad);
    let lpad = ((w - text_w) / 2).max(0) as u32;

    {
        let mut drw = get_drw().clone();
        if let Some(ref ss) = ctx.g.cfg.statusscheme {
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

/// Draw the shutdown/power-off button that appears after the layout indicator
/// when no client is selected on the monitor.
///
/// The button is `bh` pixels wide and renders a small power-icon made from
/// filled rectangles so it is visible without a font glyph.  Returns the new
/// x offset (i.e. `x + bh`).
pub(crate) fn draw_shutdown_button(ctx: &WmCtx, x: i32, bh: i32) -> i32 {
    let mut drw = get_drw().clone();

    // Use the status scheme as the base colours.
    if let Some(ref ss) = ctx.g.cfg.statusscheme {
        let scheme = ColorScheme {
            fg: ss.fg.clone(),
            bg: ss.bg.clone(),
            detail: ss.detail.clone(),
        };
        drw.set_scheme(scheme);
    }

    // Background fill for the button cell.
    drw.rect(x, 0, bh as u32, bh as u32, true, true);

    // Draw a simple power icon using raw X11 rectangles so we don't need a
    // special font glyph.  The icon is centred inside the `bh × bh` cell.
    //
    //  Layout (all values relative to cell origin `x, 0`):
    //    • A vertical "stem" bar:  2 px wide, upper-centre of the icon.
    //    • A "C"-shaped arc approximated by three rectangles that form the
    //      left, bottom and right sides of a circle outline.
    //
    //  We keep the icon proportional to bh so it looks right at any bar height.

    let icon_size = bh * 5 / 8; // overall icon bounding box
    let icon_x = x + (bh - icon_size) / 2;
    let icon_y = (bh - icon_size) / 2;

    let stroke = (icon_size / 6).max(2); // stroke thickness
    let gap = stroke; // gap at the top for the stem notch

    // Stem: centred horizontally, sits in the top portion of the icon.
    let stem_w = stroke;
    let stem_h = icon_size / 2;
    let stem_x = icon_x + (icon_size - stem_w) / 2;
    let stem_y = icon_y;

    // Arc approximation – three sides of a hollow circle:
    //   left bar, right bar, bottom bar.
    let arc_x = icon_x;
    let arc_y = icon_y + gap + stroke; // start below the stem gap
    let arc_w = stroke;
    let arc_h = icon_size - gap - stroke; // height of side bars
    let bot_x = icon_x + stroke;
    let bot_y = icon_y + icon_size - stroke;
    let bot_w = (icon_size - stroke * 2).max(0);
    let bot_h = stroke;

    unsafe {
        let display = drw.display();
        let drawable = drw.drawable();
        let gc = drw.gc();

        // Retrieve the foreground (fg) pixel from the current scheme.
        let fg_pixel = get_scheme_pixel(&drw, crate::drw::COL_FG);

        crate::drw::XSetForeground(display, gc, fg_pixel);

        // Stem
        crate::drw::XFillRectangle(
            display,
            drawable,
            gc,
            stem_x,
            stem_y,
            stem_w as u32,
            stem_h as u32,
        );

        // Left side of arc
        crate::drw::XFillRectangle(
            display,
            drawable,
            gc,
            arc_x,
            arc_y,
            arc_w as u32,
            arc_h as u32,
        );

        // Right side of arc
        crate::drw::XFillRectangle(
            display,
            drawable,
            gc,
            arc_x + icon_size - stroke,
            arc_y,
            arc_w as u32,
            arc_h as u32,
        );

        // Bottom of arc
        crate::drw::XFillRectangle(
            display,
            drawable,
            gc,
            bot_x,
            bot_y,
            bot_w as u32,
            bot_h as u32,
        );
    }

    x + bh
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

fn get_scheme_pixel(drw: &Drw, idx: usize) -> std::os::raw::c_ulong {
    if let Some(scheme) = drw.get_scheme() {
        if scheme.len() > idx {
            return scheme[idx].pixel() as std::os::raw::c_ulong;
        }
    }
    0
}

fn draw_window_title(m: &Monitor, c: &Client, x: i32, width: i32, bh: i32) -> Option<u32> {
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
        return Some(m.monitor_rect.x as u32 + x as u32);
    }

    None
}

pub(crate) fn draw_window_titles(m: &Monitor, x: i32, w: i32, n: i32, bh: i32) -> Option<u32> {
    let g = get_globals();
    let selected = m.selected_tags();
    let mut new_activeoffset = None;

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
        let wins: Vec<x11rb::protocol::xproto::Window> = m
            .iter_clients(&g.clients)
            .filter_map(|(c_win, c)| c.is_visible_on_tags(selected).then_some(c_win))
            .collect();

        for c_win in wins {
            let Some(c) = g.clients.get(&c_win) else {
                continue;
            };
            if !c.is_visible_on_tags(selected) {
                continue;
            }

            let c = c.clone();
            let this_width = if remainder > 0 {
                remainder -= 1;
                each_width + 1
            } else {
                each_width
            };

            if let Some(offset) = draw_window_title(m, &c, x, this_width, bh) {
                new_activeoffset = Some(offset);
            }
            x += this_width;
        }
        return new_activeoffset;
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
    None
}
