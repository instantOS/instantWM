use crate::contexts::CoreCtx;
use crate::systray::get_systray_width;
use crate::types::{Monitor, Rect};

pub(crate) const MAX_COMMAND_OFFSETS: usize = 20;
pub(crate) const TEXT_PADDING: i32 = 6;

#[derive(Debug, Clone)]
enum StatusItem {
    Text(String),
    SetBg(String),
    SetFg(String),
    ResetColors,
    Rect(Rect),
    Offset(i32),
    CommandOffset,
}

#[derive(Debug, Clone, Copy)]
struct StatusLayout {
    draw_start_x: i32,
    total_width: i32,
}

pub(crate) fn draw_status_bar(
    ctx: &mut CoreCtx,
    m: &Monitor,
    bar_height: i32,
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> (i32, i32) {
    let stext = ctx.g.status_text.clone();
    if stext.is_empty() {
        return (0, 0);
    }

    let items = parse_status_items(stext.as_bytes());
    let layout = measure_layout(ctx, m, &items, painter);

    draw_items(painter, m, bar_height, &items, layout, ctx.g, ctx.bar);

    (layout.draw_start_x, layout.total_width)
}

fn parse_status_items(bytes: &[u8]) -> Vec<StatusItem> {
    let mut items = Vec::new();
    let mut i = 0usize;
    let mut text_start = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'^' {
            i += 1;
            continue;
        }

        if i > text_start {
            let text = std::str::from_utf8(&bytes[text_start..i]).unwrap_or("");
            if !text.is_empty() {
                items.push(StatusItem::Text(text.to_string()));
            }
        }

        i += 1;
        if i >= bytes.len() {
            break;
        }

        if bytes[i] == b'^' {
            items.push(StatusItem::Text("^".to_string()));
            i += 1;
            text_start = i;
            continue;
        }

        let cmd = bytes[i];
        i += 1;

        match cmd {
            b'c' => {
                if i + 7 <= bytes.len() {
                    if let Ok(color) = std::str::from_utf8(&bytes[i..i + 7]) {
                        items.push(StatusItem::SetBg(color.to_string()));
                    }
                    i += 7;
                }
            }
            b't' => {
                if i + 7 <= bytes.len() {
                    if let Ok(color) = std::str::from_utf8(&bytes[i..i + 7]) {
                        items.push(StatusItem::SetFg(color.to_string()));
                    }
                    i += 7;
                }
            }
            b'd' => items.push(StatusItem::ResetColors),
            b'f' => items.push(StatusItem::Offset(parse_number(bytes, &mut i))),
            b'o' => items.push(StatusItem::CommandOffset),
            b'r' => {
                let x = parse_number(bytes, &mut i);
                consume_comma(bytes, &mut i);
                let y = parse_number(bytes, &mut i);
                consume_comma(bytes, &mut i);
                let w = parse_number(bytes, &mut i);
                consume_comma(bytes, &mut i);
                let h = parse_number(bytes, &mut i);
                items.push(StatusItem::Rect(Rect { x, y, w, h }));
            }
            _ => {}
        }

        if i < bytes.len() && bytes[i] == b'^' {
            i += 1;
        }
        text_start = i;
    }

    if text_start < bytes.len() {
        let text = std::str::from_utf8(&bytes[text_start..]).unwrap_or("");
        if !text.is_empty() {
            items.push(StatusItem::Text(text.to_string()));
        }
    }

    items
}

fn consume_comma(bytes: &[u8], i: &mut usize) {
    if *i < bytes.len() && bytes[*i] == b',' {
        *i += 1;
    }
}

fn parse_number(bytes: &[u8], i: &mut usize) -> i32 {
    let start = *i;
    while *i < bytes.len() && (bytes[*i].is_ascii_digit() || bytes[*i] == b'-') {
        *i += 1;
    }
    if *i > start {
        std::str::from_utf8(&bytes[start..*i])
            .ok()
            .and_then(|n| n.parse::<i32>().ok())
            .unwrap_or(0)
    } else {
        0
    }
}

fn measure_layout(
    ctx: &CoreCtx,
    m: &Monitor,
    items: &[StatusItem],
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> StatusLayout {
    let mut width = 0i32;

    for item in items {
        match item {
            StatusItem::Text(text) => width += painter.text_width(text),
            StatusItem::Offset(offset) => width += *offset,
            _ => {}
        }
    }

    let draw_width = (width + 2).max(0);
    let systray_w = if ctx.g.cfg.showsystray {
        get_systray_width(ctx) as i32
    } else {
        0
    };
    let draw_start_x = m.work_rect.w - draw_width - systray_w;

    StatusLayout {
        draw_start_x,
        total_width: width.max(0),
    }
}

fn draw_items(
    painter: &mut dyn crate::bar::paint::BarPainter,
    m: &Monitor,
    bar_height: i32,
    items: &[StatusItem],
    layout: StatusLayout,
    g: &crate::globals::Globals,
    bar: &mut crate::bar::BarState,
) {
    let Some(mut scheme) = crate::bar::theme::status_scheme(g) else {
        return;
    };
    let base_scheme = scheme.clone();

    painter.set_scheme(scheme.clone());

    let draw_width = (layout.total_width + 2).max(0);
    if draw_width > 0 {
        painter.rect(layout.draw_start_x, 0, draw_width, bar_height, true, true);
    }

    let _ = MAX_COMMAND_OFFSETS;
    bar.clear_command_offsets();

    let mut x = layout.draw_start_x + 1;
    let mut marker_idx = 0usize;

    for item in items {
        match item {
            StatusItem::Text(text) => {
                let seg_w = painter.text_width(text);
                if seg_w > 0 {
                    painter.text(x, 0, seg_w, bar_height, 0, text, false, 0);
                }
                x += seg_w;
            }
            StatusItem::Offset(offset) => x += *offset,
            StatusItem::SetBg(color) => {
                if let Some(clr) = crate::bar::theme::rgba_from_config(color) {
                    scheme.bg = clr;
                    painter.set_scheme(scheme.clone());
                }
            }
            StatusItem::SetFg(color) => {
                if let Some(clr) = crate::bar::theme::rgba_from_config(color) {
                    scheme.fg = clr;
                    painter.set_scheme(scheme.clone());
                }
            }
            StatusItem::ResetColors => {
                scheme = base_scheme.clone();
                painter.set_scheme(scheme.clone());
            }
            StatusItem::Rect(r) => {
                let rw = (r.w).max(0) as u32;
                let rh = (r.h).max(0) as u32;
                if rw > 0 && rh > 0 {
                    painter.rect(x + r.x, r.y, rw as i32, rh as i32, true, false);
                }
            }
            StatusItem::CommandOffset => {
                if marker_idx < MAX_COMMAND_OFFSETS {
                    bar.command_offsets[marker_idx] = x;
                    marker_idx += 1;
                }
            }
        }
    }

    if marker_idx < MAX_COMMAND_OFFSETS {
        bar.command_offsets[marker_idx] = -1;
    }

    let _ = m;
}
