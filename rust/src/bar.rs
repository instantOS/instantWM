use crate::config::{SchemeClose, SchemeHover, SchemeTag, SchemeWin};
use crate::drw::{Clr, Drw, COL_BG, COL_DETAIL, COL_FG};
use crate::globals::{get_globals, get_globals_mut};
use crate::systray::{get_systray_width, update_systray};
use crate::types::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::Window;

static DRAW_BAR_RECURSION: AtomicUsize = AtomicUsize::new(0);

const DETAIL_BAR_HEIGHT_NORMAL: i32 = 4;
const DETAIL_BAR_HEIGHT_HOVER: i32 = 8;
const STARTMENU_ICON_SIZE: i32 = 14;
const STARTMENU_ICON_INNER: i32 = 6;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub static mut PAUSEDRAW: bool = false;
pub static mut COMMANDOFFSETS: [i32; 20] = [-1; 20];

// Helper function to parse hex color to u32 (0x00RRGGBB format for x11rb)
fn parse_color_to_u32(color: &str) -> u32 {
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

// Helper to get color from scheme
fn get_scheme_color(scheme: &[Clr], col_idx: usize) -> u32 {
    if let Some(clr) = scheme.get(col_idx) {
        (clr.color.pixel & 0x00FFFFFF) as u32
    } else {
        0x121212
    }
}

pub fn text_width(text: &str) -> i32 {
    let g = get_globals();
    if let Some(ref drw) = g.drw {
        let mut drw = drw.clone();
        drw.fontset_getwidth(text) as i32
    } else {
        0
    }
}

pub fn get_layout_symbol_width(m: &MonitorInner) -> i32 {
    let ltsymbol = unsafe { std::str::from_utf8_unchecked(&m.ltsymbol) };
    ((text_width(ltsymbol) + get_lrpad()) as f32 * 1.5) as i32
}

pub fn click_status(_arg: &Arg) {
    let mut x: i32 = 0;
    let mut y: i32 = 0;
    if !get_root_ptr(&mut x, &mut y) {
        return;
    }

    let mut i: i32 = 0;
    loop {
        if i > 19
            || unsafe { COMMANDOFFSETS[i as usize] == -1 }
            || unsafe { COMMANDOFFSETS[i as usize] == 0 }
        {
            break;
        }
        let g = get_globals();
        if let Some(selmon) = &g.selmon {
            let mon = &g.monitors[*selmon];
            if x - mon.mx < unsafe { COMMANDOFFSETS[i as usize] } {
                break;
            }
        }
        i += 1;
    }
}

pub fn draw_status_bar(m: &mut MonitorInner, bh: i32, stext: &[u8]) -> i32 {
    let stext_str = match std::str::from_utf8(stext) {
        Ok(s) if !s.is_empty() && s.chars().next() != Some('\0') => s,
        _ => return 0,
    };

    let text = stext_str.to_string();
    let mut is_code = false;
    let mut w: i32 = 0;

    let bytes = text.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos] == b'^' {
            if !is_code {
                is_code = true;
                let segment = std::str::from_utf8(&bytes[..pos]).unwrap_or("");
                w += (text_width(segment) - get_lrpad()).max(0);
                pos += 1;
                if pos < bytes.len() && bytes[pos] == b'f' {
                    pos += 1;
                    let mut num_end = pos;
                    while num_end < bytes.len() && bytes[num_end].is_ascii_digit() {
                        num_end += 1;
                    }
                    if num_end > pos {
                        if let Ok(num) = std::str::from_utf8(&bytes[pos..num_end]) {
                            if let Ok(val) = num.parse::<i32>() {
                                w += val;
                            }
                        }
                    }
                }
            } else {
                is_code = false;
            }
        }
        pos += 1;
    }
    if !is_code {
        let segment = std::str::from_utf8(&bytes[pos.saturating_sub(1)..]).unwrap_or("");
        w += (text_width(segment) - get_lrpad()).max(0);
    }

    w = w.max(0);

    {
        let mut g = get_globals_mut();
        g.statuswidth = w;
    }

    w = (w + 2).max(0);
    let stw = get_systray_width() as i32;
    let ret = m.ww - w - stw;
    let mut x = ret;

    {
        let g = get_globals();
        if let Some(ref drw) = g.drw {
            let mut drw = drw.clone();
            if let Some(ref scheme) = g.statusscheme {
                drw.set_scheme(scheme.clone());
            }
            if w > 0 {
                drw.rect(x, 0, w as u32, bh as u32, true, true);
            }
        }
    }
    x += 1;

    let mut cmd_counter: i32 = 0;
    let mut custom_color = false;
    let mut text_pos = 0;
    is_code = false;

    while text_pos < bytes.len() {
        if bytes[text_pos] == b'^' && !is_code {
            is_code = true;

            let segment = std::str::from_utf8(&bytes[..text_pos]).unwrap_or("");
            let seg_w = (text_width(segment) - get_lrpad()).max(0);

            if seg_w > 0 {
                draw_text_at(x, 0, seg_w as u32, bh as u32, 0, segment, false, 0);
            }
            x += seg_w;
            text_pos += 1;

            while text_pos < bytes.len() && bytes[text_pos] != b'^' {
                match bytes[text_pos] {
                    b'c' => {
                        text_pos += 1;
                        if text_pos + 6 < bytes.len() {
                            let color = std::str::from_utf8(&bytes[text_pos..text_pos + 7]);
                            if let Ok(color_str) = color {
                                custom_color = true;
                                set_bg_color(color_str);
                            }
                            text_pos += 7;
                        }
                    }
                    b't' => {
                        text_pos += 1;
                        if text_pos + 6 < bytes.len() {
                            let color = std::str::from_utf8(&bytes[text_pos..text_pos + 7]);
                            if let Ok(color_str) = color {
                                custom_color = true;
                                set_fg_color(color_str);
                            }
                            text_pos += 7;
                        }
                    }
                    b'd' => {
                        reset_status_colors();
                        text_pos += 1;
                    }
                    b'r' => {
                        text_pos += 1;
                        let rx = parse_next_number(&bytes[text_pos..]);
                        while text_pos < bytes.len() && bytes[text_pos] != b',' {
                            text_pos += 1;
                        }
                        text_pos += 1;
                        let ry = parse_next_number(&bytes[text_pos..]);
                        while text_pos < bytes.len() && bytes[text_pos] != b',' {
                            text_pos += 1;
                        }
                        text_pos += 1;
                        let rw = parse_next_number(&bytes[text_pos..]);
                        while text_pos < bytes.len() && bytes[text_pos] != b',' {
                            text_pos += 1;
                        }
                        text_pos += 1;
                        let rh = parse_next_number(&bytes[text_pos..]);

                        draw_rect_at(rx + x, ry, rw as u32, rh as u32, true, false);
                    }
                    b'f' => {
                        text_pos += 1;
                        let offset = parse_next_number(&bytes[text_pos..]);
                        x += offset;
                    }
                    b'o' => {
                        if cmd_counter <= 20 {
                            unsafe {
                                COMMANDOFFSETS[cmd_counter as usize] = x;
                            }
                            cmd_counter += 1;
                        }
                        text_pos += 1;
                    }
                    _ => {
                        text_pos += 1;
                    }
                }
            }

            if text_pos < bytes.len() {
                text_pos += 1;
            }
            is_code = false;
        } else {
            text_pos += 1;
        }
    }

    if custom_color {
        reset_status_colors();
    }

    if cmd_counter < 20 {
        if cmd_counter == 0 {
            unsafe {
                COMMANDOFFSETS[0] = -1;
            }
        } else {
            unsafe {
                COMMANDOFFSETS[cmd_counter as usize + 1] = -1;
            }
        }
    }

    if !is_code {
        let remaining = std::str::from_utf8(&bytes[text_pos.saturating_sub(1)..]).unwrap_or("");
        let seg_w = (text_width(remaining) - get_lrpad()).max(0);
        if seg_w > 0 {
            draw_text_at(x, 0, seg_w as u32, bh as u32, 0, remaining, false, 0);
        }
    }

    ret
}

fn parse_next_number(bytes: &[u8]) -> i32 {
    let mut pos = 0;
    let start = pos;
    while pos < bytes.len() && (bytes[pos].is_ascii_digit() || bytes[pos] == b'-') {
        pos += 1;
    }
    if pos > start {
        std::str::from_utf8(&bytes[start..pos])
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    } else {
        0
    }
}

fn set_bg_color(color: &str) {
    let g = get_globals();
    if let Some(ref drw) = g.drw {
        let mut drw = drw.clone();
        if let Ok(clr) = drw.clr_create(color) {
            if let Some(ref scheme) = g.statusscheme {
                let mut new_scheme = scheme.clone();
                if new_scheme.len() > COL_BG {
                    new_scheme[COL_BG] = clr;
                    drw.set_scheme(new_scheme);
                }
            }
        }
    }
}

fn set_fg_color(color: &str) {
    let g = get_globals();
    if let Some(ref drw) = g.drw {
        let mut drw = drw.clone();
        if let Ok(clr) = drw.clr_create(color) {
            if let Some(ref scheme) = g.statusscheme {
                let mut new_scheme = scheme.clone();
                if new_scheme.len() > COL_FG {
                    new_scheme[COL_FG] = clr;
                    drw.set_scheme(new_scheme);
                }
            }
        }
    }
}

fn reset_status_colors() {
    let g = get_globals();
    let statusbarcolors = &g.statusbarcolors;
    if statusbarcolors.len() >= 2 {
        if let Some(ref drw) = g.drw {
            let mut drw = drw.clone();
            if let (Ok(fg), Ok(bg)) = (
                drw.clr_create(statusbarcolors[0]),
                drw.clr_create(statusbarcolors[1]),
            ) {
                let detail = bg.clone();
                let scheme = vec![fg, bg, detail];
                drw.set_scheme(scheme);
            }
        }
    }
}

fn draw_text_at(
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    lpad: u32,
    text: &str,
    invert: bool,
    detail_height: i32,
) {
    let g = get_globals();
    if let Some(ref drw) = g.drw {
        let mut drw = drw.clone();
        if let Some(ref scheme) = g.statusscheme {
            drw.set_scheme(scheme.clone());
        }
        drw.text(x, y, w, h, lpad, text, invert, detail_height);
    }
}

fn draw_rect_at(x: i32, y: i32, w: u32, h: u32, filled: bool, invert: bool) {
    let g = get_globals();
    if let Some(ref drw) = g.drw {
        let mut drw = drw.clone();
        if let Some(ref scheme) = g.statusscheme {
            drw.set_scheme(scheme.clone());
        }
        drw.rect(x, y, w, h, filled, invert);
    }
}

pub fn draw_startmenu_icon(bh: i32) {
    let g = get_globals();
    let icon_offset = (bh - CLOSE_BUTTON_WIDTH) / 2;
    let startmenu_invert = if let Some(selmon_idx) = g.selmon {
        let mon = &g.monitors[selmon_idx];
        mon.gesture == Gesture::StartMenu
    } else {
        false
    };

    let startmenu_size = g.startmenusize as i32;

    // Get colors from scheme, matching C version
    let scheme = if g.tagprefix {
        let schemes = &g.tagschemes;
        if schemes.len() > SchemeHover::NoHover as usize {
            let hover_idx = SchemeHover::NoHover as usize;
            if schemes[hover_idx].len() > SchemeTag::Focus as usize {
                Some(schemes[hover_idx][SchemeTag::Focus as usize].clone())
            } else {
                g.statusscheme.clone()
            }
        } else {
            g.statusscheme.clone()
        }
    } else {
        g.statusscheme.clone()
    };

    let Some(ref scheme) = scheme else { return };

    if let Some(ref drw) = g.drw {
        let mut drw = drw.clone();
        drw.set_scheme(scheme.clone());

        // Background rectangle
        drw.rect(
            0,
            0,
            startmenu_size as u32,
            bh as u32,
            true,
            !startmenu_invert,
        );
        // Outer icon square
        drw.rect(
            5,
            icon_offset,
            STARTMENU_ICON_SIZE as u32,
            STARTMENU_ICON_SIZE as u32,
            true,
            startmenu_invert,
        );
        // Inner icon square 1
        drw.rect(
            9,
            icon_offset + 4,
            STARTMENU_ICON_INNER as u32,
            STARTMENU_ICON_INNER as u32,
            true,
            !startmenu_invert,
        );
        // Inner icon square 2
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

pub fn get_tag_scheme(
    m: &MonitorInner,
    i: u32,
    occupied_tags: u32,
    is_hover: bool,
) -> Option<Vec<Clr>> {
    let g = get_globals();
    let hover_idx = if is_hover {
        SchemeHover::Hover as usize
    } else {
        SchemeHover::NoHover as usize
    };

    let schemes = &g.tagschemes;
    if schemes.len() <= hover_idx {
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

        let is_selected = g.selmon.map_or(false, |selmon_idx| {
            g.monitors.get(selmon_idx).map_or(false, |selmon| {
                std::ptr::eq(m as *const _, selmon as *const _)
            })
        });

        if is_selected && sel_has_tag {
            return schemes[hover_idx].get(SchemeTag::Focus as usize).cloned();
        }
        if m.tagset[m.seltags as usize] & (1 << i) != 0 {
            return schemes[hover_idx].get(SchemeTag::NoFocus as usize).cloned();
        }
        if m.showtags == 0 {
            return schemes[hover_idx].get(SchemeTag::Filled as usize).cloned();
        }
        return schemes[hover_idx]
            .get(SchemeTag::Inactive as usize)
            .cloned();
    }

    if m.tagset[m.seltags as usize] & (1 << i) != 0 {
        return schemes[hover_idx].get(SchemeTag::Empty as usize).cloned();
    }
    schemes[hover_idx]
        .get(SchemeTag::Inactive as usize)
        .cloned()
}

pub fn draw_tag_indicators(
    m: &mut MonitorInner,
    mut x: i32,
    occupied_tags: u32,
    urg: u32,
    bh: i32,
) -> i32 {
    let g = get_globals();
    let lrpad = g.lrpad;
    let show_alt_tag = g.showalttag;
    let bar_dragging = g.bar_dragging;
    let num_tags = g.numtags;

    let tags = g.tags;
    let tags_alt = g.tagsalt.clone();

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

        if m.showtags != 0 {
            if occupied_tags & (1 << actual_i) == 0
                && m.tagset[m.seltags as usize] & (1 << actual_i) == 0
            {
                continue;
            }
        }

        let tag_name = if actual_i < tags.len() as u32 {
            let tag_bytes = &tags[actual_i as usize];
            let len = tag_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(tag_bytes.len());
            std::str::from_utf8(&tag_bytes[..len]).unwrap_or("")
        } else {
            ""
        };

        let display_name = if show_alt_tag && (actual_i as usize) < tags_alt.len() {
            tags_alt[actual_i as usize]
        } else {
            tag_name
        };

        let w = text_width(display_name) + lrpad;

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
                    // Use filled scheme when dragging over a tag
                    let schemes = &g.tagschemes;
                    if schemes.len() > SchemeHover::Hover as usize {
                        if let Some(s) =
                            schemes[SchemeHover::Hover as usize].get(SchemeTag::Filled as usize)
                        {
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
                    (lrpad / 2) as u32,
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

pub fn draw_layout_indicator(m: &MonitorInner, mut x: i32, bh: i32) -> i32 {
    let g = get_globals();
    let w = get_layout_symbol_width(m);

    let ltsymbol = unsafe { std::str::from_utf8_unchecked(&m.ltsymbol) };
    let text_w = text_width(ltsymbol);
    let lpad = ((w - text_w) as f32 * 0.5 + 10.0) as u32;

    if let Some(ref drw) = g.drw {
        let mut drw = drw.clone();
        if let Some(ref scheme) = g.statusscheme {
            drw.set_scheme(scheme.clone());
        }
        x = drw.text(x, 0, w as u32, bh as u32, lpad, ltsymbol, false, 0);
    }

    x
}

pub fn get_window_scheme(c: &ClientInner, is_hover: bool) -> Option<Vec<Clr>> {
    let g = get_globals();
    let hover_idx = if is_hover {
        SchemeHover::Hover as usize
    } else {
        SchemeHover::NoHover as usize
    };

    let schemes = &g.windowschemes;
    if schemes.len() <= hover_idx {
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
            return schemes[hover_idx]
                .get(SchemeWin::OverlayFocus as usize)
                .cloned();
        }
        if c.issticky {
            return schemes[hover_idx]
                .get(SchemeWin::StickyFocus as usize)
                .cloned();
        }
        return schemes[hover_idx].get(SchemeWin::Focus as usize).cloned();
    }

    if is_overlay {
        return schemes[hover_idx].get(SchemeWin::Overlay as usize).cloned();
    }
    if c.issticky {
        return schemes[hover_idx].get(SchemeWin::Sticky as usize).cloned();
    }
    if is_hidden(c) {
        return schemes[hover_idx]
            .get(SchemeWin::Minimized as usize)
            .cloned();
    }
    schemes[hover_idx].get(SchemeWin::Normal as usize).cloned()
}

fn is_hidden(c: &ClientInner) -> bool {
    c.tags == 0
}

pub fn draw_close_button(c: &ClientInner, x: i32, bh: i32) {
    let g = get_globals();

    let is_hover = if let Some(selmon_idx) = g.selmon {
        let selmon = &g.monitors[selmon_idx];
        selmon.gesture != Gesture::CloseButton
    } else {
        true
    };

    let hover_idx = if is_hover {
        SchemeHover::NoHover as usize
    } else {
        SchemeHover::Hover as usize
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

        let schemes = &g.closebuttonschemes;
        if schemes.len() > hover_idx && schemes[hover_idx].len() > scheme_idx {
            drw.set_scheme(schemes[hover_idx][scheme_idx].clone());
        }

        let button_x = x + bh / 6;
        let button_y =
            (bh - CLOSE_BUTTON_WIDTH) / 2 - if is_hover { 0 } else { CLOSE_BUTTON_DETAIL };

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
                (bh - CLOSE_BUTTON_WIDTH) / 2 + CLOSE_BUTTON_HEIGHT
                    - if is_hover { 0 } else { CLOSE_BUTTON_DETAIL },
                CLOSE_BUTTON_WIDTH as u32,
                (CLOSE_BUTTON_DETAIL + if is_hover { 0 } else { CLOSE_BUTTON_DETAIL }) as u32,
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

pub fn draw_window_title(m: &mut MonitorInner, c: &ClientInner, x: i32, width: i32, bh: i32) {
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

    let client_name = unsafe { std::str::from_utf8_unchecked(&c.name) };
    let name_len = client_name.chars().take_while(|&c| c != '\0').count();
    let client_name = &client_name[..name_len.min(client_name.len())];

    let text_w = text_width(client_name);

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

pub fn draw_window_titles(m: &mut MonitorInner, x: i32, w: i32, n: i32, bh: i32) {
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
    } else {
        if let Some(ref drw) = g.drw {
            let mut drw = drw.clone();
            if let Some(ref scheme) = g.statusscheme {
                drw.set_scheme(scheme.clone());
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
                let title_width = text_width(help_text);
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
}

pub fn draw_bar(m: &mut MonitorInner) {
    let count = DRAW_BAR_RECURSION.fetch_add(1, Ordering::SeqCst);
    eprintln!("TRACE: draw_bar - recursion count = {}", count + 1);
    if count > 50 {
        eprintln!(
            "ERROR: draw_bar infinite recursion detected! Count = {}",
            count + 1
        );
        std::process::abort();
    }

    if unsafe { PAUSEDRAW } {
        DRAW_BAR_RECURSION.fetch_sub(1, Ordering::SeqCst);
        return;
    }

    if !m.showbar {
        DRAW_BAR_RECURSION.fetch_sub(1, Ordering::SeqCst);
        return;
    }

    // Resize the Drw drawable to match the bar window size
    {
        let mut g = get_globals_mut();
        let bh = g.bh;
        if let Some(ref mut drw) = g.drw {
            // Instead of resizing the pixmap, set the drawable to the window directly
            // This avoids the XCopyArea across different X11 connections
            drw.set_drawable(m.barwin);
            drw.w = m.ww as u32;
            drw.h = bh as u32;
        }
    } // Release write lock here

    let g = get_globals();
    let bh = g.bh;
    let showsystray = g.showsystray;

    eprintln!(
        "DEBUG draw_bar: m.barwin={}, m.ww={}, m.wx={}, m.by={}, bh={}",
        m.barwin, m.ww, m.wx, m.by, bh
    );
    eprintln!("DEBUG draw_bar: g.drw.is_some={}", g.drw.is_some());
    if let Some(ref drw) = g.drw {
        eprintln!(
            "DEBUG draw_bar: drw.w={}, drw.h={}, drw.drawable={}",
            drw.w,
            drw.h,
            drw.drawable()
        );
    }

    let mut stw: i32 = 0;
    if showsystray {
        if let Some(selmon_idx) = g.selmon {
            if g.monitors
                .get(selmon_idx)
                .map_or(false, |selmon| std::ptr::eq(selmon, m))
            {
                stw = get_systray_width() as i32;
            }
        }
    }

    let stext = g.stext.clone();
    let stext_str = unsafe { std::str::from_utf8_unchecked(&stext) };

    let mut sw: i32 = 0;
    if let Some(selmon_idx) = g.selmon {
        if g.monitors
            .get(selmon_idx)
            .map_or(false, |selmon| std::ptr::eq(selmon, m))
        {
            sw = m.ww - stw - draw_status_bar(m, bh, &stext);
        }
    }

    eprintln!("TRACE: draw_bar - before draw_startmenu_icon");
    draw_startmenu_icon(bh);
    eprintln!("TRACE: draw_bar - before resize_bar_win");
    resize_bar_win(m);
    eprintln!("TRACE: draw_bar - after resize_bar_win");

    eprintln!("TRACE: draw_bar - before client loop");
    let mut occupied_tags: u32 = 0;
    let mut urg: u32 = 0;
    let mut n: i32 = 0;

    for c in g.clients.values() {
        let mon_match = c.mon_id.map_or(false, |mon_id| {
            g.selmon.map_or(false, |selmon_idx| mon_id == selmon_idx)
        });

        if mon_match {
            let is_visible = crate::types::is_visible(
                c.tags,
                m.tagset[m.seltags as usize],
                m.seltags,
                c.issticky,
            );
            if is_visible {
                n += 1;
            }
            occupied_tags |= if c.tags == 255 { 0 } else { c.tags };
            if c.isurgent {
                urg |= c.tags;
            }
        }
    }
    eprintln!("TRACE: draw_bar - after client loop");

    let startmenu_size = g.startmenusize as i32;
    let mut x = startmenu_size;
    eprintln!("TRACE: draw_bar - before draw_tag_indicators");
    x = draw_tag_indicators(m, x, occupied_tags, urg, bh);
    eprintln!("TRACE: draw_bar - after draw_tag_indicators, before draw_layout_indicator");
    x = draw_layout_indicator(m, x, bh);
    eprintln!("TRACE: draw_bar - after draw_layout_indicator");

    let window_width = (m.ww - sw - x - stw).max(0);
    if window_width > bh {
        eprintln!("TRACE: draw_bar - before draw_window_titles");
        draw_window_titles(m, x, window_width, n, bh);
        eprintln!("TRACE: draw_bar - after draw_window_titles");
    }

    eprintln!("TRACE: draw_bar - before final block");

    m.bt = n;
    m.bar_clients_width = window_width;

    // Flush Xlib to ensure all Drw operations are sent to the X server
    {
        let g = get_globals();
        if let Some(ref drw) = g.drw {
            unsafe {
                crate::drw::XFlush(drw.display());
            }
        }
    }

    DRAW_BAR_RECURSION.fetch_sub(1, Ordering::SeqCst);
}

pub fn draw_bars() {
    let g = get_globals();
    let monitors = g.monitors.clone();
    for (i, _m) in monitors.iter().enumerate() {
        let mut g = get_globals_mut();
        if let Some(m) = g.monitors.get_mut(i) {
            draw_bar(m);
        }
    }
}

pub fn reset_bar() {
    let mut g = get_globals_mut();

    let should_reset = if let Some(selmon_idx) = g.selmon {
        g.monitors
            .get(selmon_idx)
            .map_or(false, |selmon| selmon.gesture != Gesture::None)
    } else {
        false
    };

    if !should_reset {
        return;
    }

    if let Some(selmon_idx) = g.selmon {
        if let Some(selmon) = g.monitors.get_mut(selmon_idx) {
            selmon.gesture = Gesture::None;
        }
    }

    if g.altcursor != AltCursor::None {
        reset_cursor();
    }

    let selmon_idx = g.selmon;
    if let Some(idx) = selmon_idx {
        let monitors = g.monitors.clone();
        if let Some(m) = monitors.get(idx) {
            let mut m = m.clone();
            draw_bar(&mut m);
        }
    }
}

fn reset_cursor() {}

pub fn update_status() {
    eprintln!("TRACE: update_status - start");
    let (root, selmon_idx, monitors) = {
        let g = get_globals();
        let root = g.root;
        let selmon_idx = g.selmon;
        let monitors = g.monitors.clone();
        (root, selmon_idx, monitors)
    }; // Read lock released here
    eprintln!("TRACE: update_status - after getting initial data");

    let text = get_text_prop(root, x11rb::protocol::xproto::AtomEnum::WM_NAME.into());
    eprintln!("TRACE: update_status - after get_text_prop");

    {
        eprintln!("TRACE: update_status - before get_globals_mut");
        let mut g = get_globals_mut();
        eprintln!("TRACE: update_status - after get_globals_mut");
        match text {
            Some(t) => {
                if t.starts_with("ipc:") {
                    return;
                }
                let bytes = t.as_bytes();
                let len = bytes.len().min(g.stext.len() - 1);
                g.stext[..len].copy_from_slice(&bytes[..len]);
                g.stext[len] = 0;
            }
            None => {
                let default_text = format!("instantwm-{}", VERSION);
                let bytes = default_text.as_bytes();
                let len = bytes.len().min(g.stext.len() - 1);
                g.stext[..len].copy_from_slice(&bytes[..len]);
                g.stext[len] = 0;
            }
        }
    } // Write lock released here

    if let Some(selmon_idx) = selmon_idx {
        if let Some(m) = monitors.get(selmon_idx) {
            let mut m = m.clone();
            draw_bar(&mut m);
        }
    }

    update_systray();
}

fn get_text_prop(_win: Window, _atom: u32) -> Option<String> {
    None
}

pub fn update_bar_pos(m: &mut MonitorInner) {
    // Pass bh as a parameter to avoid calling get_globals() which can deadlock
    // if called while holding a write lock
    let bh = {
        let g = get_globals();
        g.bh
    };

    m.wy = m.my;
    m.wh = m.mh;

    if m.showbar {
        m.wh -= bh;
        if m.topbar {
            m.by = m.wy;
            m.wy += bh;
        } else {
            m.by = m.wy + m.wh;
        }
    } else {
        m.by = -bh;
    }
}

// New version that takes bh as parameter to avoid deadlock
pub fn update_bar_pos_with_bh(m: &mut MonitorInner, bh: i32) {
    m.wy = m.my;
    m.wh = m.mh;

    if m.showbar {
        m.wh -= bh;
        if m.topbar {
            m.by = m.wy;
            m.wy += bh;
        } else {
            m.by = m.wy + m.wh;
        }
    } else {
        m.by = -bh;
    }
}

pub fn resize_bar_win(m: &MonitorInner) {
    let g = get_globals();
    let bh = g.bh;
    let showsystray = g.showsystray;

    let mut w = m.ww as u32;
    if showsystray {
        if let Some(selmon_idx) = g.selmon {
            if g.monitors
                .get(selmon_idx)
                .map_or(false, |selmon| std::ptr::eq(selmon, m))
            {
                w -= get_systray_width();
            }
        }
    }

    let x11 = crate::globals::get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.configure_window(
            m.barwin,
            &x11rb::protocol::xproto::ConfigureWindowAux::new()
                .x(m.wx as i32)
                .y(m.by as i32)
                .width(w)
                .height(bh as u32),
        );
    }
}

pub fn update_bars() {
    eprintln!("DEBUG update_bars: START");
    let (bh, showsystray, bar_configs, xlibdisplay, root) = {
        let g = get_globals();
        let bh = g.bh;
        let showsystray = g.showsystray;
        let xlibdisplay = g.xlibdisplay.0;
        let root = g.root;

        eprintln!(
            "DEBUG update_bars: monitors.len={}, xlibdisplay={:p}, root={}",
            g.monitors.len(),
            xlibdisplay,
            root
        );

        let mut bar_configs = Vec::new();
        for (i, m) in g.monitors.iter().enumerate() {
            eprintln!(
                "DEBUG update_bars: monitor {} barwin={}, showbar={}",
                i, m.barwin, m.showbar
            );
            if m.barwin != 0 {
                eprintln!(
                    "DEBUG update_bars: skipping monitor {} - barwin already set",
                    i
                );
                continue;
            }

            let mut w = m.ww as u32;
            if showsystray {
                if let Some(selmon_idx) = g.selmon {
                    if selmon_idx == i {
                        w -= crate::systray::get_systray_width() as u32;
                    }
                }
            }
            eprintln!(
                "DEBUG update_bars: adding bar config for monitor {}: wx={}, by={}, w={}, bh={}",
                i, m.wx, m.by, w, bh
            );
            bar_configs.push((i, m.wx, m.by, w, bh));
        }
        eprintln!("DEBUG update_bars: bar_configs.len={}", bar_configs.len());
        (bh, showsystray, bar_configs, xlibdisplay, root)
    };

    eprintln!(
        "DEBUG update_bars: xlibdisplay.is_null={}",
        xlibdisplay.is_null()
    );
    if xlibdisplay.is_null() {
        return;
    }

    // Use x11rb to create windows with proper attributes
    let x11 = crate::globals::get_x11();
    if let Some(ref conn) = x11.conn {
        for (i, wx, by, w, bh) in bar_configs {
            eprintln!(
                "DEBUG update_bars: creating window for monitor {}: wx={}, by={}, w={}, bh={}",
                i, wx, by, w, bh
            );

            let win_id = conn.generate_id().unwrap();

            let aux = x11rb::protocol::xproto::CreateWindowAux::new()
                .override_redirect(1) // Don't manage our own bar!
                .background_pixel(0xFF0000) // TEST: Red background
                .event_mask(
                    x11rb::protocol::xproto::EventMask::BUTTON_PRESS
                        | x11rb::protocol::xproto::EventMask::EXPOSURE
                        | x11rb::protocol::xproto::EventMask::LEAVE_WINDOW,
                );

            let _ = conn.create_window(
                x11rb::COPY_FROM_PARENT as u8,
                win_id,
                root,
                wx as i16,
                by as i16,
                w as u16,
                bh as u16,
                0,
                x11rb::protocol::xproto::WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT as u32,
                &aux,
            );

            let _ = conn.map_window(win_id);
            let _ = conn.flush();

            eprintln!(
                "DEBUG update_bars: x11rb created and mapped win_id={}",
                win_id
            );

            let mut globals_mut = crate::globals::get_globals_mut();
            globals_mut.monitors[i].barwin = win_id;
            eprintln!("DEBUG update_bars: stored barwin={} in globals", win_id);
        }
    }
    eprintln!("DEBUG update_bars: END");
}

pub fn toggle_bar(_arg: &Arg) {
    let mut g = get_globals_mut();

    let animated = g.animated;
    let client_count = g.clients.len() as i32;

    let mut tmp_no_anim = false;
    if animated && client_count > 6 {
        g.animated = false;
        tmp_no_anim = true;
    }

    if let Some(selmon_idx) = g.selmon {
        if let Some(selmon) = g.monitors.get_mut(selmon_idx) {
            selmon.showbar = !selmon.showbar;

            if let Some(ref mut pertag) = selmon.pertag {
                if (pertag.current_tag as usize) < pertag.showbars.len() {
                    pertag.showbars[pertag.current_tag as usize] = selmon.showbar;
                }
            }

            update_bar_pos(selmon);
            let m = selmon.clone();
            resize_bar_win(&m);
        }
    }

    if tmp_no_anim {
        g.animated = true;
    }
}

pub fn update_bars_for_monitors() {
    let g = get_globals();
    let bh = g.bh;
    let showsystray = g.showsystray;

    for m in g.monitors.iter() {
        let mut w = m.ww;
        if showsystray {
            w -= get_systray_width() as i32;
        }
        let _ = (w, bh);
    }
}

fn get_root_ptr(_x: &mut i32, _y: &mut i32) -> bool {
    false
}

fn get_lrpad() -> i32 {
    let g = get_globals();
    g.lrpad
}

pub fn get_tag_width() -> i32 {
    crate::tags::get_tag_width()
}

pub fn get_tag_at_x(x: i32) -> i32 {
    crate::tags::get_tag_at_x(x)
}

pub fn window_title_mouse_handler(arg: &Arg) {
    crate::mouse::window_title_mouse_handler(arg);
}

pub fn window_title_mouse_handler_right(arg: &Arg) {
    crate::mouse::window_title_mouse_handler_right(arg);
}

pub fn close_win(arg: &Arg) {
    crate::client::close_win(arg);
}

pub fn up_scale_client(arg: &Arg) {
    crate::animation::up_scale_client(arg);
}

pub fn down_scale_client(arg: &Arg) {
    crate::animation::down_scale_client(arg);
}
