use super::TEXT_PADDING;
use super::{
    I3Align, I3Block, I3ClickEvent, I3MinWidth, ParsedStatus, StatusClickTarget, StatusItem,
};
use crate::contexts::CoreCtx;
use crate::types::{Monitor, Rect};

#[derive(Debug, Clone, Copy)]
struct StatusLayout {
    draw_start_x: i32,
    total_width: i32,
}

/// Draw the status bar.
///
/// The `systray_width` parameter is pre-calculated by the caller to avoid
/// backend-specific dependencies in this function.
pub(crate) fn draw_status_bar(
    ctx: &mut CoreCtx,
    systray_width: i32,
    m: &Monitor,
    bar_height: i32,
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> (i32, i32, Vec<StatusClickTarget>) {
    let mode = ctx.globals().behavior.current_mode.clone();
    let stext_owned: String;
    let stext = if crate::overview::is_mode_name(&mode) {
        stext_owned = "mode: overview".to_string();
        stext_owned.as_str()
    } else if !mode.is_empty() && mode != "default" {
        let mode_display = ctx
            .globals()
            .cfg
            .modes
            .get(&mode)
            .and_then(|m| m.description.as_ref())
            .map(|s| s.as_str())
            .unwrap_or(&mode);
        stext_owned = format!("mode: {}", mode_display);
        stext_owned.as_str()
    } else {
        stext_owned = ctx.globals().bar_runtime.status_text.clone();
        stext_owned.as_str()
    };

    if stext.is_empty() {
        return (0, 0, Vec::new());
    }

    let items = ctx.bar.status_items_for_text(stext).to_vec();
    draw_status_items(
        systray_width,
        m,
        bar_height,
        items.as_slice(),
        ctx.globals().status_scheme(),
        painter,
    )
}

pub(crate) fn hit_test_i3_click_target(
    click_targets: &[StatusClickTarget],
    local_x: i32,
) -> Option<usize> {
    click_targets
        .iter()
        .find(|target| local_x >= target.start_x && local_x < target.end_x)
        .map(|target| target.index)
}

pub(crate) fn resolve_i3_click<'a>(
    parsed: &'a ParsedStatus,
    click_targets: &[StatusClickTarget],
    local_x: i32,
) -> Option<(&'a I3Block, StatusClickTarget)> {
    let line = parsed.i3bar.as_ref()?;
    let block_idx = hit_test_i3_click_target(click_targets, local_x)?;
    let block = line.blocks.get(block_idx)?;
    let target = click_targets
        .iter()
        .copied()
        .find(|target| target.index == block_idx)?;

    Some((block, target))
}

pub(crate) fn modifiers_from_mask(mask: u32) -> Vec<String> {
    let mut modifiers = Vec::new();

    if mask & crate::config::keybindings::SHIFT != 0 {
        modifiers.push("Shift".to_string());
    }
    if mask & crate::config::keybindings::CONTROL != 0 {
        modifiers.push("Control".to_string());
    }
    if mask & crate::config::keybindings::MOD1 != 0 {
        modifiers.push("Mod1".to_string());
    }
    if mask & crate::config::keybindings::MODKEY != 0 {
        modifiers.push("Mod4".to_string());
    }

    modifiers
}

pub(crate) fn make_i3_click_event(
    block: &I3Block,
    target: StatusClickTarget,
    button: u8,
    x: i32,
    y: i32,
    bar_height: i32,
    clean_state: u32,
) -> I3ClickEvent {
    I3ClickEvent {
        name: block.name.clone(),
        instance: block.instance.clone(),
        button,
        x,
        y,
        relative_x: x - target.start_x,
        relative_y: y.max(0),
        width: (target.end_x - target.start_x).max(0),
        height: bar_height.max(0),
        modifiers: modifiers_from_mask(clean_state),
    }
}

pub(crate) fn emit_i3bar_status_click(
    parsed: &ParsedStatus,
    click_targets: &[StatusClickTarget],
    local_x: i32,
    y: i32,
    button: u8,
    bar_height: i32,
    clean_state: u32,
) -> bool {
    let Some((block, target)) = resolve_i3_click(parsed, click_targets, local_x) else {
        return false;
    };

    super::runtime::enqueue_i3bar_click_event(make_i3_click_event(
        block,
        target,
        button,
        local_x,
        y,
        bar_height,
        clean_state,
    ));
    true
}

fn measure_layout(
    systray_width: i32,
    m: &Monitor,
    items: &[StatusItem],
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> StatusLayout {
    let mut width = 0i32;

    for item in items {
        match item {
            StatusItem::Text(text) => width += painter.text_width(text),
            StatusItem::I3Block(block) => width += measure_i3_block_width(block, painter),
        }
    }

    let draw_width = (width + 2).max(0);
    let draw_start_x = m.work_rect.w - draw_width - systray_width;

    StatusLayout {
        draw_start_x,
        total_width: width.max(0),
    }
}

fn measure_i3_block_width(block: &I3Block, painter: &mut dyn crate::bar::paint::BarPainter) -> i32 {
    let text = block_render_text(block);
    let text_width = painter.text_width(text);

    let min_width = match &block.min_width {
        Some(I3MinWidth::Text(s)) => painter.text_width(s),
        Some(I3MinWidth::Pixels(px)) => *px,
        None => 0,
    };

    let padding = if !block.separator && block.separator_block_width == 0 {
        0
    } else {
        TEXT_PADDING
    };

    let content_width = text_width.max(min_width).max(0);
    let border_width = block.border_left + block.border_right;
    let block_width = border_width + padding * 2 + content_width;

    let separator_width = if block.separator {
        block.separator_block_width
    } else {
        0
    };

    (block_width + separator_width).max(0)
}

pub(crate) fn draw_status_items(
    systray_width: i32,
    m: &Monitor,
    bar_height: i32,
    items: &[StatusItem],
    base_scheme: crate::bar::paint::BarScheme,
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> (i32, i32, Vec<StatusClickTarget>) {
    let layout = measure_layout(systray_width, m, items, painter);
    let mut click_targets = Vec::new();
    draw_items(
        painter,
        bar_height,
        items,
        layout,
        &base_scheme,
        &mut click_targets,
    );
    (layout.draw_start_x, layout.total_width, click_targets)
}

fn draw_items(
    painter: &mut dyn crate::bar::paint::BarPainter,
    bar_height: i32,
    items: &[StatusItem],
    layout: StatusLayout,
    base_scheme: &crate::bar::paint::BarScheme,
    click_targets: &mut Vec<StatusClickTarget>,
) {
    painter.set_scheme(base_scheme.clone());

    let draw_width = (layout.total_width + 2).max(0);
    if draw_width > 0 {
        painter.rect(
            Rect::new(layout.draw_start_x, 0, draw_width, bar_height),
            true,
            true,
        );
    }

    click_targets.clear();

    let mut x = layout.draw_start_x + 1;
    let mut click_idx = 0usize;

    for item in items {
        match item {
            StatusItem::Text(text) => {
                let seg_w = painter.text_width(text);
                if seg_w > 0 {
                    painter.text(Rect::new(x, 0, seg_w, bar_height), 0, text, false, 0);
                }
                x += seg_w;
            }
            StatusItem::I3Block(block) => {
                let total_w = draw_i3_block(painter, x, bar_height, block, base_scheme);
                if total_w > 0 {
                    click_targets.push(StatusClickTarget {
                        start_x: x,
                        end_x: x + total_w,
                        index: click_idx,
                    });
                }
                x += total_w;
                click_idx += 1;
            }
        }
    }
}

fn draw_i3_block(
    painter: &mut dyn crate::bar::paint::BarPainter,
    x: i32,
    bar_height: i32,
    block: &I3Block,
    base_scheme: &crate::bar::paint::BarScheme,
) -> i32 {
    let mut fg = block
        .color
        .as_deref()
        .and_then(crate::bar::theme::rgba_from_config)
        .unwrap_or(base_scheme.fg);
    let mut bg = block
        .background
        .as_deref()
        .and_then(crate::bar::theme::rgba_from_config)
        .unwrap_or(base_scheme.bg);
    let mut detail = base_scheme.detail;

    if block.urgent {
        std::mem::swap(&mut fg, &mut bg);
        detail = fg;
    }

    let block_scheme = crate::bar::paint::BarScheme { fg, bg, detail };
    painter.set_scheme(block_scheme.clone());

    let text = block_render_text(block);
    let text_width = painter.text_width(text);
    let min_width = match &block.min_width {
        Some(I3MinWidth::Text(s)) => painter.text_width(s),
        Some(I3MinWidth::Pixels(px)) => *px,
        None => 0,
    };

    let padding = if !block.separator && block.separator_block_width == 0 {
        0
    } else {
        TEXT_PADDING
    };

    let content_width = text_width.max(min_width).max(0);
    let block_inner_width = (padding * 2 + content_width).max(0);
    let block_width = (block.border_left + block.border_right + block_inner_width).max(0);
    if block_width <= 0 {
        return 0;
    }

    painter.rect(Rect::new(x, 0, block_width, bar_height), true, true);

    let border_color = block
        .border
        .as_deref()
        .and_then(crate::bar::theme::rgba_from_config)
        .unwrap_or(block_scheme.detail);

    let border_scheme = crate::bar::paint::BarScheme {
        fg: border_color,
        bg: block_scheme.bg,
        detail: border_color,
    };
    painter.set_scheme(border_scheme);

    if block.border_top > 0 {
        painter.rect(
            Rect::new(x, 0, block_width, block.border_top.min(bar_height)),
            true,
            false,
        );
    }
    if block.border_bottom > 0 {
        let h = block.border_bottom.min(bar_height);
        painter.rect(Rect::new(x, bar_height - h, block_width, h), true, false);
    }
    if block.border_left > 0 {
        painter.rect(
            Rect::new(x, 0, block.border_left.min(block_width), bar_height),
            true,
            false,
        );
    }
    if block.border_right > 0 {
        let w = block.border_right.min(block_width);
        painter.rect(
            Rect::new(x + block_width - w, 0, w, bar_height),
            true,
            false,
        );
    }

    painter.set_scheme(block_scheme);

    let text_area_x = x + block.border_left + padding;
    let text_area_width =
        (block_width - block.border_left - block.border_right - padding * 2).max(0);

    if text_area_width > 0 {
        let lpad = match block.align {
            I3Align::Left => 0,
            I3Align::Center => ((text_area_width - text_width) / 2).max(0),
            I3Align::Right => (text_area_width - text_width).max(0),
        };
        painter.text(
            Rect::new(text_area_x, 0, text_area_width, bar_height),
            lpad,
            text,
            false,
            0,
        );
    }

    let separator_width = if block.separator {
        block.separator_block_width
    } else {
        0
    };

    if separator_width > 0 {
        painter.set_scheme(base_scheme.clone());
        painter.rect(
            Rect::new(x + block_width, 0, separator_width, bar_height),
            true,
            true,
        );

        let line_h = (bar_height - 8).max(4);
        let line_y = (bar_height - line_h) / 2;
        let line_x = x + block_width + separator_width / 2;
        painter.rect(Rect::new(line_x, line_y, 1, line_h), true, false);
    }

    block_width + separator_width
}

fn block_render_text(block: &I3Block) -> &str {
    block
        .short_text
        .as_deref()
        .unwrap_or(block.full_text.as_str())
}
