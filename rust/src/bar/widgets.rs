use crate::config::{SchemeClose, SchemeTag, SchemeWin};
use crate::drw::{Drw, COL_BG, COL_DETAIL};
use crate::globals::get_globals;
use crate::types::*;

const DETAIL_BAR_HEIGHT_NORMAL: i32 = 4;
const DETAIL_BAR_HEIGHT_HOVER: i32 = 8;
const STARTMENU_ICON_SIZE: i32 = 14;
const STARTMENU_ICON_INNER: i32 = 6;

pub(crate) fn draw_startmenu_icon(bh: i32) {
    let g = get_globals();
    let icon_offset = (bh - CLOSE_BUTTON_WIDTH) / 2;
    let startmenu_invert = if let Some(selmon_idx) = g.selmon {
        let mon = &g.monitors[selmon_idx];
        mon.gesture == Gesture::StartMenu
    } else {
        false
    };

    let startmenu_size = g.startmenusize;
    let scheme: Option<ColorScheme> = if g.tags.prefix {
        let schemes = &g.tags.schemes;
        if !schemes.no_hover.is_empty() {
            schemes.no_hover.get(SchemeTag::Focus as usize).cloned()
        } else {
            g.statusscheme.as_ref().map(|s| ColorScheme {
                fg: s.fg.clone(),
                bg: s.bg.clone(),
                detail: s.detail.clone(),
            })
        }
    } else {
        g.statusscheme.as_ref().map(|s| ColorScheme {
            fg: s.fg.clone(),
            bg: s.bg.clone(),
            detail: s.detail.clone(),
        })
    };

    let Some(ref scheme) = scheme else { return };

    if let Some(ref drw) = g.drw {
        let mut drw = drw.clone();
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
}

fn get_tag_scheme(
    m: &MonitorInner,
    i: u32,
    occupied_tags: u32,
    is_hover: bool,
) -> Option<ColorScheme> {
    let g = get_globals();
    let schemes = if is_hover {
        &g.tags.schemes.hover
    } else {
        &g.tags.schemes.no_hover
    };

    if schemes.is_empty() {
        return None;
    }

    if occupied_tags & (1 << i) != 0 {
        let sel_has_tag = g.selmon.map_or(false, |selmon_idx| {
            g.monitors
                .get(selmon_idx)
                .and_then(|selmon| {
                    selmon
                        .sel
                        .and_then(|sel_win| g.clients.get(&sel_win).map(|c| c.tags & (1 << i) != 0))
                })
                .unwrap_or(false)
        });

        let is_selected = g
            .selmon
            .and_then(|selmon_idx| g.monitors.get(selmon_idx))
            .map_or(false, |selmon| selmon.num == m.num);

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

pub(crate) fn draw_tag_indicators(
    m: &mut MonitorInner,
    mut x: i32,
    occupied_tags: u32,
    urg: u32,
    bh: i32,
) -> i32 {
    let g = get_globals();
    let lrpad = g.lrpad;
    let show_alt_tag = g.tags.show_alt;
    let bar_dragging = g.bar_dragging;
    let num_tags = g.tags.count;

    let tags_alt = g.tags.alt_names.clone();

    for i in 0..num_tags as u32 {
        if i >= 9 {
            continue;
        }

        let is_hover = if let Some(selmon_idx) = g.selmon {
            let selmon = &g.monitors[selmon_idx];
            selmon.gesture as u32 == i + 1
        } else {
            false
        };

        let current_tag = m.pertag.as_ref().map(|p| p.current_tag).unwrap_or(0);
        let actual_i = if i == 8 && current_tag > 9 {
            current_tag - 1
        } else {
            i
        };

        if m.showtags != 0
            && occupied_tags & (1 << actual_i) == 0
            && m.tagset[m.seltags as usize] & (1 << actual_i) == 0
        {
            continue;
        }

        let tag_name = g
            .tags
            .names
            .get(actual_i as usize)
            .map(|s| s.as_str())
            .unwrap_or("");

        let display_name = if show_alt_tag && (actual_i as usize) < tags_alt.len() {
            tags_alt[actual_i as usize]
        } else {
            tag_name
        };

        let text_w = super::text_width(display_name);
        let w = text_w + lrpad;
        let lpad = ((w - text_w) / 2).max(0) as u32;

        if let Some(scheme) = get_tag_scheme(m, actual_i, occupied_tags, is_hover) {
            if let Some(ref drw) = g.drw {
                let mut drw = drw.clone();
                let detail_height = if is_hover {
                    DETAIL_BAR_HEIGHT_HOVER
                } else {
                    DETAIL_BAR_HEIGHT_NORMAL
                };

                let mut draw_scheme = scheme.clone();
                if is_hover && bar_dragging {
                    let schemes = &g.tags.schemes.hover;
                    if !schemes.is_empty() {
                        if let Some(s) = schemes.get(SchemeTag::Filled as usize) {
                            draw_scheme = s.clone();
                        }
                    }
                }
                drw.set_scheme(draw_scheme);

                let is_urgent = urg & (1 << actual_i) != 0;
                x = drw.text(
                    x,
                    0,
                    w as u32,
                    bh as u32,
                    lpad,
                    display_name,
                    is_urgent,
                    detail_height,
                );
            }
        } else {
            x += w;
        }
    }
    x
}

pub(crate) fn draw_layout_indicator(m: &MonitorInner, mut x: i32, bh: i32) -> i32 {
    let g = get_globals();
    let lrpad = g.lrpad;
    let ltsymbol = super::layout_symbol(m);
    let text_w = super::text_width(ltsymbol);
    let w = (text_w + lrpad).max(lrpad);
    let lpad = ((w - text_w) / 2).max(0) as u32;

    if let Some(ref drw) = g.drw {
        let mut drw = drw.clone();
        if let Some(ref ss) = g.statusscheme {
            let scheme = ColorScheme {
                fg: ss.fg.clone(),
                bg: ss.bg.clone(),
                detail: ss.detail.clone(),
            };
            drw.set_scheme(scheme);
        }
        x = drw.text(x, 0, w as u32, bh as u32, lpad, ltsymbol, false, 0);
    }

    x
}

fn get_window_scheme(c: &ClientInner, is_hover: bool) -> Option<ColorScheme> {
    let g = get_globals();
    let schemes = if is_hover {
        &g.windowschemes.hover
    } else {
        &g.windowschemes.no_hover
    };

    if schemes.is_empty() {
        return None;
    }

    let is_selected = g.selmon.map_or(false, |selmon_idx| {
        g.monitors.get(selmon_idx).map_or(false, |selmon| {
            selmon.sel.map_or(false, |sel_win| sel_win == c.win)
        })
    });

    let is_overlay = g.selmon.map_or(false, |selmon_idx| {
        g.monitors.get(selmon_idx).map_or(false, |selmon| {
            selmon
                .overlay
                .map_or(false, |overlay_win| overlay_win == c.win)
        })
    });

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
    if c.tags == 0 {
        return schemes.get(SchemeWin::Minimized as usize).cloned();
    }
    schemes.get(SchemeWin::Normal as usize).cloned()
}

pub(crate) fn draw_close_button(c: &ClientInner, x: i32, bh: i32) {
    let g = get_globals();

    let close_hovered = if let Some(selmon_idx) = g.selmon {
        let selmon = &g.monitors[selmon_idx];
        selmon.gesture == Gesture::CloseButton
    } else {
        false
    };

    let schemes = if close_hovered {
        &g.closebuttonschemes.hover
    } else {
        &g.closebuttonschemes.no_hover
    };

    if let Some(ref drw) = g.drw {
        let mut drw = drw.clone();

        let scheme_idx = if c.islocked {
            SchemeClose::Locked as usize
        } else if g.selmon.map_or(false, |selmon_idx| {
            g.monitors
                .get(selmon_idx)
                .and_then(|selmon| {
                    selmon.sel.and_then(|sel_win| {
                        g.clients
                            .get(&sel_win)
                            .map(|sel_c| sel_c.is_fullscreen && sel_c.win == c.win)
                    })
                })
                .unwrap_or(false)
        }) {
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

fn draw_window_title(m: &mut MonitorInner, c: &ClientInner, x: i32, width: i32, bh: i32) {
    let g = get_globals();

    let is_hover = if let Some(selmon_idx) = g.selmon {
        let selmon = &g.monitors[selmon_idx];
        selmon.gesture == Gesture::None
            && selmon.sel.map_or(false, |sel_win| {
                g.clients
                    .get(&sel_win)
                    .map_or(false, |hover_c| hover_c.win == c.win)
            })
    } else {
        false
    };

    let client_name = c.name.as_str();
    let text_w = super::text_width(client_name);

    if let Some(ref drw) = g.drw {
        let mut drw = drw.clone();
        if let Some(scheme) = get_window_scheme(c, is_hover) {
            drw.set_scheme(scheme);
        }

        let lpad = if text_w < width - 64 {
            ((width - text_w) as f32 * 0.5) as u32
        } else {
            (g.lrpad / 2 + 20) as u32
        };

        drw.text(x, 0, width as u32, bh as u32, lpad, client_name, false, 4);
    }

    let is_selected = g.selmon.map_or(false, |selmon_idx| {
        g.monitors.get(selmon_idx).map_or(false, |selmon| {
            selmon.sel.map_or(false, |sel_win| sel_win == c.win)
        })
    });

    if is_selected {
        draw_close_button(c, x, bh);
        m.activeoffset = m.mx as u32 + x as u32;
    }
}

pub(crate) fn draw_window_titles(m: &mut MonitorInner, x: i32, w: i32, n: i32, bh: i32) {
    let g = get_globals();

    if n > 0 {
        let total_width = w + 1;
        let each_width = total_width / n;
        let mut remainder = total_width % n;
        let mut x = x;

        let clients: Vec<ClientInner> = g.clients.values().cloned().collect();
        for c in clients.iter() {
            let mon_match = c.mon_id.map_or(false, |mon_id| {
                g.selmon.map_or(false, |selmon_idx| mon_id == selmon_idx)
            });
            if !mon_match {
                continue;
            }

            let is_visible = crate::types::is_visible(
                c.tags,
                m.tagset[m.seltags as usize],
                m.seltags,
                c.issticky,
            );
            if !is_visible {
                continue;
            }

            let this_width = if remainder > 0 {
                remainder -= 1;
                each_width + 1
            } else {
                each_width
            };

            draw_window_title(m, c, x, this_width, bh);
            x += this_width;
        }
        return;
    }

    if let Some(ref drw) = g.drw {
        let mut drw = drw.clone();
        if let Some(ref ss) = g.statusscheme {
            let scheme = ColorScheme {
                fg: ss.fg.clone(),
                bg: ss.bg.clone(),
                detail: ss.detail.clone(),
            };
            drw.set_scheme(scheme);
        }
        drw.rect(x, 0, w as u32, bh as u32, true, true);
        drw.text(
            x,
            0,
            bh as u32,
            bh as u32,
            (g.lrpad / 2) as u32,
            "",
            false,
            0,
        );

        let has_clients = g.selmon.map_or(false, |selmon_idx| {
            g.monitors
                .get(selmon_idx)
                .map_or(false, |selmon| selmon.clients.is_some())
        });

        if !has_clients {
            let help_text = "Press space to launch an application";
            let title_width = super::text_width(help_text);
            let bar_clients_width = m.bar_clients_width;
            let title_width = if title_width < bar_clients_width - bh {
                title_width
            } else {
                bar_clients_width - bh
            };
            drw.text(
                x + bh + ((bar_clients_width - bh - title_width + 1) / 2),
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
