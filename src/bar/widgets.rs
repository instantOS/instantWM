use crate::bar::{TagHitRange, TitleHitRange};
use crate::contexts::CoreCtx;
use crate::types::*;

const DETAIL_BAR_HEIGHT_NORMAL: i32 = 4;
const DETAIL_BAR_HEIGHT_HOVER: i32 = 8;
const STARTMENU_ICON_SIZE: i32 = 14;
const STARTMENU_ICON_INNER: i32 = 6;

pub(crate) fn draw_startmenu_icon(
    ctx: &CoreCtx,
    bar_height: i32,
    painter: &mut dyn crate::bar::paint::BarPainter,
) {
    let icon_offset = (bar_height - CLOSE_BUTTON_WIDTH) / 2;
    let startmenu_invert = ctx.globals().selected_monitor().gesture == Gesture::StartMenu;

    let startmenu_size = ctx.globals().cfg.startmenusize;
    let scheme = ctx.globals().status_scheme();

    painter.set_scheme(scheme);

    painter.rect(0, 0, startmenu_size, bar_height, true, !startmenu_invert);
    painter.rect(
        5,
        icon_offset,
        STARTMENU_ICON_SIZE,
        STARTMENU_ICON_SIZE,
        true,
        startmenu_invert,
    );
    painter.rect(
        9,
        icon_offset + 4,
        STARTMENU_ICON_INNER,
        STARTMENU_ICON_INNER,
        true,
        !startmenu_invert,
    );
    painter.rect(
        19,
        icon_offset + STARTMENU_ICON_SIZE,
        STARTMENU_ICON_INNER,
        STARTMENU_ICON_INNER,
        true,
        startmenu_invert,
    );
}
pub(crate) fn draw_tag_indicators(
    ctx: &mut CoreCtx,
    m: &Monitor,
    mut x: i32,
    occupied_tags: TagMask,
    urg: TagMask,
    bar_height: i32,
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> i32 {
    let horizontal_padding = ctx.globals().cfg.horizontal_padding;
    let lpad = (horizontal_padding / 2) as u32;
    let bar_dragging = ctx.globals().drag.bar_active;

    let tags = crate::tags::bar::visible_tags_ctx(ctx, m, occupied_tags);

    let selmon_gesture = ctx.globals().selected_monitor().gesture;

    for t in &tags {
        // A tag cell is hovered when the current gesture is Tag(slot) for this cell's slot.
        let is_hover = selmon_gesture == Gesture::Tag(t.slot);

        let text_w = painter.text_width(t.label);
        let width = (text_w + horizontal_padding).max(horizontal_padding);
        ctx.bar.cache_tag_width(t.slot, width);

        let scheme = ctx
            .globals()
            .tag_scheme(m, t.tag_index as u32, occupied_tags, urg, is_hover);

        let mut draw_scheme = scheme;
        if is_hover && bar_dragging {
            draw_scheme = ctx.globals().tag_hover_fill_scheme();
        }
        painter.set_scheme(draw_scheme);

        let detail_height = if is_hover {
            DETAIL_BAR_HEIGHT_HOVER
        } else {
            DETAIL_BAR_HEIGHT_NORMAL
        };

        x = painter.text(
            x,
            0,
            width,
            bar_height,
            lpad as i32,
            t.label,
            urg.contains(t.tag_index + 1),
            detail_height,
        );

        if let Some(hit) = ctx.bar.monitor_hit_cache_mut(m.id()) {
            hit.tag_ranges.push(TagHitRange {
                start: x - width,
                end: x,
                tag_index: t.tag_index,
            });
        }
    }
    ctx.bar.tag_strip_width = x;

    x
}

pub(crate) fn draw_layout_indicator(
    ctx: &mut CoreCtx,
    m: &Monitor,
    mut x: i32,
    bar_height: i32,
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> i32 {
    let horizontal_padding = ctx.globals().cfg.horizontal_padding;
    let ltsymbol = m.layout_symbol();
    let text_w = painter.text_width(&ltsymbol);
    ctx.bar.layout_symbol_width = text_w;
    let w = (text_w + horizontal_padding).max(horizontal_padding);
    let lpad = ((w - text_w) / 2).max(0);

    painter.set_scheme(ctx.globals().status_scheme());
    let start_x = x;
    x = painter.text(x, 0, w, bar_height, lpad, &ltsymbol, false, 0);

    if let Some(hit) = ctx.bar.monitor_hit_cache_mut(m.id()) {
        hit.layout_start = start_x;
        hit.layout_end = x;
    }

    x
}

/// Draw the shutdown/power-off button that appears after the layout indicator
/// when no client is selected on the monitor.
///
/// The button is `bar_height` pixels wide and renders a small power-icon made from
/// filled rectangles so it is visible without a font glyph.  Returns the new
/// x offset (i.e. `x + bar_height`).
pub(crate) fn draw_shutdown_button(
    ctx: &CoreCtx,
    x: i32,
    bar_height: i32,
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> i32 {
    // Use the status scheme as the base colours.
    painter.set_scheme(ctx.globals().status_scheme());

    // Background fill for the button cell.
    painter.rect(x, 0, bar_height, bar_height, true, true);

    // Draw a simple power icon using raw X11 rectangles so we don't need a
    // special font glyph.  The icon is centred inside the `bar_height × bar_height` cell.
    //
    //  Layout (all values relative to cell origin `x, 0`):
    //    • A vertical "stem" bar:  2 px wide, upper-centre of the icon.
    //    • A "C"-shaped arc approximated by three rectangles that form the
    //      left, bottom and right sides of a circle outline.
    //
    //  We keep the icon proportional to bar_height so it looks right at any bar height.

    let icon_size = bar_height * 5 / 8; // overall icon bounding box
    let icon_x = x + (bar_height - icon_size) / 2;
    let icon_y = (bar_height - icon_size) / 2;

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

    // Stem
    painter.rect(stem_x, stem_y, stem_w, stem_h, true, false);
    // Left side of arc
    painter.rect(arc_x, arc_y, arc_w, arc_h, true, false);
    // Right side of arc
    painter.rect(arc_x + icon_size - stroke, arc_y, arc_w, arc_h, true, false);
    // Bottom of arc
    painter.rect(bot_x, bot_y, bot_w, bot_h, true, false);

    x + bar_height
}
pub(crate) fn draw_close_button(
    ctx: &CoreCtx,
    c: &Client,
    x: i32,
    bar_height: i32,
    painter: &mut dyn crate::bar::paint::BarPainter,
) {
    let selmon = ctx.globals().selected_monitor();
    let close_hovered = selmon.gesture == Gesture::CloseButton;
    let is_fullscreen = selmon
        .sel
        .and_then(|selected_window| {
            ctx.globals()
                .clients
                .get(&selected_window)
                .map(|sel_c| sel_c.is_fullscreen && sel_c.win == c.win)
        })
        .unwrap_or(false);

    let mut scheme = ctx
        .globals()
        .close_button_scheme(close_hovered, c.is_locked, is_fullscreen);
    // Use the scheme detail color for the lower accent bar (matches intended darker tone).
    scheme.fg = scheme.detail;
    painter.set_scheme(scheme);

    let button_x = x + bar_height / 6;
    let detail_offset = if close_hovered {
        CLOSE_BUTTON_DETAIL
    } else {
        0
    };
    let button_y = (bar_height - CLOSE_BUTTON_WIDTH) / 2 - detail_offset;

    painter.rect(
        button_x,
        button_y,
        CLOSE_BUTTON_WIDTH,
        CLOSE_BUTTON_HEIGHT,
        true,
        true,
    );
    painter.rect(
        button_x,
        (bar_height - CLOSE_BUTTON_WIDTH) / 2 + CLOSE_BUTTON_HEIGHT - detail_offset,
        CLOSE_BUTTON_WIDTH,
        CLOSE_BUTTON_DETAIL + detail_offset,
        true,
        false,
    );
}

fn draw_window_title(
    ctx: &mut CoreCtx,
    m: &Monitor,
    c: &Client,
    x: i32,
    width: i32,
    bar_height: i32,
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> Option<u32> {
    let selected_monitor = ctx.globals().selected_monitor();
    let is_hover = selected_monitor.gesture == Gesture::WinTitle(c.win);

    let client_name = c.name.as_str();
    let text_w = painter.text_width(client_name);

    painter.set_scheme(ctx.globals().window_scheme(c, is_hover));

    let lpad = if text_w < width - 64 {
        ((width - text_w) as f32 * 0.5) as i32
    } else {
        ctx.globals().cfg.horizontal_padding / 2 + if width >= 32 { 20 } else { 0 }
    };

    painter.text(x, 0, width, bar_height, lpad, client_name, false, 4);

    let is_selected = selected_monitor.sel == Some(c.win);

    if is_selected {
        if width >= 32 {
            draw_close_button(ctx, c, x, bar_height, painter);
        }
        return Some(m.monitor_rect.x as u32 + x as u32);
    }

    None
}

pub(crate) fn draw_window_titles(
    ctx: &mut CoreCtx,
    m: &Monitor,
    x: i32,
    w: i32,
    n: i32,
    bar_height: i32,
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> Option<u32> {
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
        let wins: Vec<WindowId> = m
            .iter_clients(ctx.globals().clients.map())
            .filter_map(|(c_win, c)| c.is_visible(selected).then_some(c_win))
            .collect();

        for c_win in wins {
            let Some(c) = ctx.globals().clients.get(&c_win) else {
                continue;
            };
            if !c.is_visible(selected) {
                continue;
            }

            let c = c.clone();
            let this_width = if remainder > 0 {
                remainder -= 1;
                each_width + 1
            } else {
                each_width
            };

            if let Some(offset) = draw_window_title(ctx, m, &c, x, this_width, bar_height, painter)
            {
                new_activeoffset = Some(offset);
            }

            if let Some(hit) = ctx.bar.monitor_hit_cache_mut(m.id()) {
                hit.title_ranges.push(TitleHitRange {
                    start: x,
                    end: x + this_width,
                    win: c.win,
                });
            }
            x += this_width;
        }
        return new_activeoffset;
    }

    painter.set_scheme(ctx.globals().status_scheme());
    painter.rect(x, 0, w, bar_height, true, true);

    let has_clients = !m.clients.is_empty();

    if !has_clients {
        let help_text = "Press space to launch an application";
        let text_w = painter.text_width(help_text);
        let avail = w - bar_height;
        let title_width = text_w.min(avail);
        painter.text(
            x + bar_height + ((avail - title_width + 1) / 2),
            0,
            title_width,
            bar_height,
            0,
            help_text,
            false,
            0,
        );
    }
    None
}
