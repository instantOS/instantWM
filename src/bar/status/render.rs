use super::TEXT_PADDING;
use super::{
    I3Align, I3Block, I3ClickEvent, I3MinWidth, ParsedStatus, StatusClickTarget, StatusItem,
};
use crate::bar::color::Rgba;
use crate::bar::paint::{BarPainter, BarScheme};
use crate::types::{Point, Rect};

const HOVER_INDICATOR_HEIGHT: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct StatusBlockHover {
    pub block_index: usize,
    pub color: Rgba,
}

#[derive(Debug, Default)]
pub(crate) struct StatusRenderOutput {
    /// Visible status area in bar-local coordinates.
    pub bounds: Rect,
    pub click_targets: Vec<StatusClickTarget>,
}

/// The same pointer location expressed in the coordinate spaces required by
/// the i3bar click protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StatusClickGeometry {
    pub root_position: Point,
    pub output_position: Point,
    pub bar_position: Point,
}

#[derive(Debug, Clone, Copy)]
struct BlockMetrics {
    width: i32,
    text_width: i32,
    padding: i32,
}

#[derive(Debug, Clone, Copy)]
enum MeasuredItem {
    Text {
        width: i32,
    },
    I3Block {
        full: Option<BlockMetrics>,
        short: Option<BlockMetrics>,
        has_short: bool,
        separator_width: i32,
    },
}

impl MeasuredItem {
    fn metrics(self, use_short: bool) -> Option<BlockMetrics> {
        match self {
            Self::Text { .. } => None,
            Self::I3Block {
                full,
                short,
                has_short,
                ..
            } => {
                if use_short && has_short {
                    short
                } else {
                    full
                }
            }
        }
    }

    fn width(self, use_short: bool) -> i32 {
        match self {
            Self::Text { width } => width,
            Self::I3Block { .. } => self.metrics(use_short).map_or(0, |metrics| metrics.width),
        }
    }

    fn is_visible(self, use_short: bool) -> bool {
        self.width(use_short) > 0
    }
}

#[derive(Debug, Clone, Copy)]
enum LaidOutItem {
    Text {
        item_index: usize,
        bounds: Rect,
    },
    I3Block {
        item_index: usize,
        block_index: usize,
        bounds: Rect,
        text_bounds: Rect,
        text_lpad: i32,
        separator_bounds: Option<Rect>,
        use_short: bool,
    },
}

#[derive(Debug, Default)]
struct StatusLayout {
    clip_bounds: Rect,
    items: Vec<LaidOutItem>,
}

pub(crate) fn hit_test_i3_click_target(
    click_targets: &[StatusClickTarget],
    bar_position: Point,
) -> Option<usize> {
    click_targets
        .iter()
        .find(|target| target.bounds.contains_point(bar_position))
        .map(|target| target.block_index)
}

pub(crate) fn resolve_i3_click<'a>(
    parsed: &'a ParsedStatus,
    click_targets: &[StatusClickTarget],
    bar_position: Point,
) -> Option<(&'a I3Block, StatusClickTarget)> {
    let line = parsed.i3bar.as_ref()?;
    let block_index = hit_test_i3_click_target(click_targets, bar_position)?;
    let block = line.blocks.get(block_index)?;
    let target = click_targets
        .iter()
        .copied()
        .find(|target| target.block_index == block_index)?;

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
    if mask & u32::from(x11rb::protocol::xproto::ModMask::M2) != 0 {
        modifiers.push("Mod2".to_string());
    }
    if mask & u32::from(x11rb::protocol::xproto::ModMask::M3) != 0 {
        modifiers.push("Mod3".to_string());
    }
    if mask & crate::config::keybindings::MODKEY != 0 {
        modifiers.push("Mod4".to_string());
    }
    if mask & u32::from(x11rb::protocol::xproto::ModMask::M5) != 0 {
        modifiers.push("Mod5".to_string());
    }

    modifiers
}

pub(crate) fn make_i3_click_event(
    block: &I3Block,
    target: StatusClickTarget,
    button: u8,
    geometry: StatusClickGeometry,
    clean_state: u32,
) -> I3ClickEvent {
    let relative_position = Point::new(
        geometry.bar_position.x - target.bounds.x,
        geometry.bar_position.y - target.bounds.y,
    );

    I3ClickEvent {
        name: block.name.clone(),
        instance: block.instance.clone(),
        button,
        x: geometry.root_position.x,
        y: geometry.root_position.y,
        relative_x: relative_position.x,
        relative_y: relative_position.y,
        output_x: geometry.output_position.x,
        output_y: geometry.output_position.y,
        width: target.bounds.w.max(0),
        height: target.bounds.h.max(0),
        modifiers: modifiers_from_mask(clean_state),
    }
}

pub(crate) fn emit_i3bar_status_click(
    parsed: &ParsedStatus,
    click_targets: &[StatusClickTarget],
    geometry: StatusClickGeometry,
    button: u8,
    clean_state: u32,
) -> bool {
    let Some((block, target)) = resolve_i3_click(parsed, click_targets, geometry.bar_position)
    else {
        return false;
    };

    super::runtime::enqueue_i3bar_click_event(make_i3_click_event(
        block,
        target,
        button,
        geometry,
        clean_state,
    ));
    true
}

fn block_text(block: &I3Block, use_short: bool) -> &str {
    if use_short {
        block
            .short_text
            .as_deref()
            .unwrap_or(block.full_text.as_str())
    } else {
        block.full_text.as_str()
    }
}

fn measure_i3_block_variant(
    block: &I3Block,
    text: &str,
    min_width: i32,
    painter: &mut dyn BarPainter,
) -> Option<BlockMetrics> {
    if text.is_empty() {
        return None;
    }

    let text_width = painter.text_width(text).max(0);
    let padding = if !block.separator && block.separator_block_width == 0 {
        0
    } else {
        TEXT_PADDING
    };
    let natural_width = text_width
        .saturating_add(block.border_widths.horizontal())
        .saturating_add(padding.saturating_mul(2));

    Some(BlockMetrics {
        width: natural_width.max(min_width),
        text_width,
        padding,
    })
}

fn measure_items(items: &[StatusItem], painter: &mut dyn BarPainter) -> Vec<MeasuredItem> {
    items
        .iter()
        .map(|item| match item {
            StatusItem::Text(text) => MeasuredItem::Text {
                width: if text.is_empty() {
                    0
                } else {
                    painter.text_width(text).max(0)
                },
            },
            StatusItem::I3Block(block) => {
                let min_width = match &block.min_width {
                    Some(I3MinWidth::Text(text)) => painter.text_width(text).max(0),
                    Some(I3MinWidth::Pixels(pixels)) => (*pixels).max(0),
                    None => 0,
                };
                MeasuredItem::I3Block {
                    full: measure_i3_block_variant(
                        block,
                        block.full_text.as_str(),
                        min_width,
                        painter,
                    ),
                    short: block
                        .short_text
                        .as_deref()
                        .and_then(|text| measure_i3_block_variant(block, text, min_width, painter)),
                    has_short: block.short_text.is_some(),
                    separator_width: block.separator_block_width.max(0),
                }
            }
        })
        .collect()
}

fn measured_width(items: &[MeasuredItem], choices: &[bool]) -> i32 {
    let mut width = 0i32;
    let mut has_later_visible_item = false;

    for (item_index, item) in items.iter().copied().enumerate().rev() {
        let item_width = item.width(choices[item_index]);
        if item_width <= 0 {
            continue;
        }
        if has_later_visible_item
            && let MeasuredItem::I3Block {
                separator_width, ..
            } = item
        {
            width = width.saturating_add(separator_width);
        }
        width = width.saturating_add(item_width);
        has_later_visible_item = true;
    }

    width
}

fn choose_short_texts(
    items: &[StatusItem],
    measured: &[MeasuredItem],
    max_content_width: i32,
) -> Vec<bool> {
    let mut choices = vec![false; items.len()];
    let mut width = measured_width(measured, &choices);

    for item_index in 0..items.len() {
        if width <= max_content_width {
            break;
        }
        let StatusItem::I3Block(block) = &items[item_index] else {
            continue;
        };
        if block.short_text.is_none() {
            continue;
        }

        let previous_choices = choices.clone();
        choices[item_index] = true;
        if let Some(name) = block.name.as_deref() {
            for (other_index, other) in items.iter().enumerate() {
                let StatusItem::I3Block(other) = other else {
                    continue;
                };
                if other.name.as_deref() == Some(name) && other.short_text.is_some() {
                    choices[other_index] = true;
                }
            }
        }
        let shortened_width = measured_width(measured, &choices);
        if shortened_width < width {
            width = shortened_width;
        } else {
            choices = previous_choices;
        }
    }

    choices
}

fn measure_layout(
    available_bounds: Rect,
    items: &[StatusItem],
    painter: &mut dyn BarPainter,
) -> StatusLayout {
    let available_bounds = Rect::new(
        available_bounds.x,
        available_bounds.y,
        available_bounds.w.max(0),
        available_bounds.h.max(0),
    );
    let measured = measure_items(items, painter);
    let choices = choose_short_texts(items, &measured, (available_bounds.w - 2).max(0));
    let total_width = measured_width(&measured, &choices);
    if total_width <= 0 || available_bounds.w <= 0 || available_bounds.h <= 0 {
        return StatusLayout::default();
    }

    let background_width = total_width.saturating_add(2);
    let right = available_bounds.x.saturating_add(available_bounds.w);
    let background_bounds = Rect::new(
        right.saturating_sub(background_width),
        available_bounds.y,
        background_width,
        available_bounds.h,
    );
    let clip_bounds = background_bounds
        .intersection(&available_bounds)
        .unwrap_or_default();
    let mut laid_out = Vec::with_capacity(items.len());
    let mut x = background_bounds.x.saturating_add(1);
    let mut block_index = 0usize;
    let last_visible_item = measured
        .iter()
        .copied()
        .enumerate()
        .rfind(|(index, item)| item.is_visible(choices[*index]))
        .map(|(index, _)| index);

    for (item_index, (item, measured_item)) in items.iter().zip(&measured).enumerate() {
        match (item, *measured_item) {
            (StatusItem::Text(_), MeasuredItem::Text { width }) => {
                if width > 0 {
                    laid_out.push(LaidOutItem::Text {
                        item_index,
                        bounds: Rect::new(x, available_bounds.y, width, available_bounds.h),
                    });
                    x = x.saturating_add(width);
                }
            }
            (StatusItem::I3Block(block), MeasuredItem::I3Block { .. }) => {
                let use_short = choices[item_index];
                let Some(metrics) = measured_item.metrics(use_short) else {
                    block_index += 1;
                    continue;
                };
                let bounds = Rect::new(x, available_bounds.y, metrics.width, available_bounds.h);
                let text_area_x = x
                    .saturating_add(block.border_widths.left)
                    .saturating_add(metrics.padding);
                let text_area_width = metrics
                    .width
                    .saturating_sub(block.border_widths.horizontal())
                    .saturating_sub(metrics.padding.saturating_mul(2))
                    .max(0);
                let text_lpad = match block.align {
                    I3Align::Left => 0,
                    I3Align::Center => ((text_area_width - metrics.text_width) / 2).max(0),
                    I3Align::Right => (text_area_width - metrics.text_width).max(0),
                };
                x = x.saturating_add(metrics.width);

                let separator_bounds =
                    if Some(item_index) != last_visible_item && block.separator_block_width > 0 {
                        let bounds = Rect::new(
                            x,
                            available_bounds.y,
                            block.separator_block_width,
                            available_bounds.h,
                        );
                        x = x.saturating_add(block.separator_block_width);
                        Some(bounds)
                    } else {
                        None
                    };

                laid_out.push(LaidOutItem::I3Block {
                    item_index,
                    block_index,
                    bounds,
                    text_bounds: Rect::new(
                        text_area_x,
                        available_bounds.y,
                        text_area_width,
                        available_bounds.h,
                    ),
                    text_lpad,
                    separator_bounds,
                    use_short,
                });
                block_index += 1;
            }
            _ => unreachable!("status item and its measurements must have matching variants"),
        }
    }

    StatusLayout {
        clip_bounds,
        items: laid_out,
    }
}

pub(crate) fn draw_status_items(
    available_bounds: Rect,
    items: &[StatusItem],
    base_scheme: BarScheme,
    hover: Option<StatusBlockHover>,
    painter: &mut dyn BarPainter,
) -> StatusRenderOutput {
    let layout = measure_layout(available_bounds, items, painter);
    if layout.clip_bounds.w <= 0 || layout.clip_bounds.h <= 0 {
        return StatusRenderOutput::default();
    }

    painter.set_scheme(base_scheme.clone());
    painter.rect(layout.clip_bounds, true, true);

    let mut click_targets = Vec::new();
    for item in layout.items {
        match item {
            LaidOutItem::Text { item_index, bounds } => {
                let Some(bounds) = bounds.intersection(&layout.clip_bounds) else {
                    continue;
                };
                let StatusItem::Text(text) = &items[item_index] else {
                    continue;
                };
                painter.set_scheme(base_scheme.clone());
                painter.text(bounds, 0, text, false, 0);
            }
            LaidOutItem::I3Block {
                item_index,
                block_index,
                bounds,
                text_bounds,
                text_lpad,
                separator_bounds,
                use_short,
            } => {
                let Some(bounds) = fully_visible(bounds, layout.clip_bounds) else {
                    continue;
                };
                let StatusItem::I3Block(block) = &items[item_index] else {
                    continue;
                };
                draw_i3_block(
                    painter,
                    bounds,
                    text_bounds,
                    text_lpad,
                    block_text(block, use_short),
                    block,
                    &base_scheme,
                );
                if let Some(hover) = hover.filter(|hover| hover.block_index == block_index) {
                    let color = hover.color;
                    let height = HOVER_INDICATOR_HEIGHT.min(bounds.h).max(0);
                    painter.set_scheme(BarScheme {
                        foreground: color,
                        background: color,
                        detail: color,
                    });
                    painter.rect(
                        Rect::new(bounds.x, bounds.bottom() - height, bounds.w, height),
                        true,
                        false,
                    );
                }
                click_targets.push(StatusClickTarget {
                    bounds,
                    block_index,
                });

                if let Some(separator_bounds) = separator_bounds {
                    draw_separator(painter, separator_bounds, block.separator, &base_scheme);
                }
            }
        }
    }

    StatusRenderOutput {
        bounds: layout.clip_bounds,
        click_targets,
    }
}

fn fully_visible(bounds: Rect, clip_bounds: Rect) -> Option<Rect> {
    let visible = bounds.intersection(&clip_bounds)?;
    (visible == bounds).then_some(bounds)
}

fn draw_i3_block(
    painter: &mut dyn BarPainter,
    bounds: Rect,
    text_bounds: Rect,
    text_lpad: i32,
    text: &str,
    block: &I3Block,
    base_scheme: &BarScheme,
) {
    let mut foreground = block
        .color
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(base_scheme.foreground);
    let mut background = block
        .background
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(base_scheme.background);
    let mut detail = base_scheme.detail;

    if block.urgent {
        std::mem::swap(&mut foreground, &mut background);
        detail = foreground;
    }

    let block_scheme = BarScheme {
        foreground,
        background,
        detail,
    };
    painter.set_scheme(block_scheme.clone());
    painter.rect(bounds, true, true);

    let border_color = block
        .border
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(block_scheme.detail);
    painter.set_scheme(BarScheme {
        foreground: border_color,
        background: block_scheme.background,
        detail: border_color,
    });

    let border = block.border_widths;
    if border.top > 0 {
        painter.rect(
            Rect::new(bounds.x, bounds.y, bounds.w, border.top.min(bounds.h)),
            true,
            false,
        );
    }
    if border.bottom > 0 {
        let height = border.bottom.min(bounds.h);
        painter.rect(
            Rect::new(bounds.x, bounds.y + bounds.h - height, bounds.w, height),
            true,
            false,
        );
    }
    if border.left > 0 {
        painter.rect(
            Rect::new(bounds.x, bounds.y, border.left.min(bounds.w), bounds.h),
            true,
            false,
        );
    }
    if border.right > 0 {
        let width = border.right.min(bounds.w);
        painter.rect(
            Rect::new(bounds.x + bounds.w - width, bounds.y, width, bounds.h),
            true,
            false,
        );
    }

    if text_bounds.w > 0 {
        painter.set_scheme(block_scheme);
        painter.text(text_bounds, text_lpad, text, false, 0);
    }
}

fn draw_separator(
    painter: &mut dyn BarPainter,
    bounds: Rect,
    draw_line: bool,
    base_scheme: &BarScheme,
) {
    painter.set_scheme(base_scheme.clone());
    painter.rect(bounds, true, true);
    if !draw_line || bounds.w <= 0 || bounds.h <= 0 {
        return;
    }

    let line_height = (bounds.h - 8).max(1).min(bounds.h);
    let line_y = bounds.y + (bounds.h - line_height) / 2;
    let line_x = bounds.x + bounds.w / 2;
    painter.rect(Rect::new(line_x, line_y, 1, line_height), true, false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Insets;

    #[derive(Default)]
    struct RecordingPainter {
        texts: Vec<String>,
        measurement_calls: usize,
        scheme: Option<BarScheme>,
        rectangles: Vec<(Rect, Rgba)>,
    }

    impl BarPainter for RecordingPainter {
        fn text_width(&mut self, text: &str) -> i32 {
            self.measurement_calls += 1;
            text.chars().count() as i32 * 10
        }

        fn set_scheme(&mut self, scheme: BarScheme) {
            self.scheme = Some(scheme);
        }

        fn rect(&mut self, bounds: Rect, _filled: bool, invert: bool) {
            let color = self
                .scheme
                .as_ref()
                .expect("drawing requires a color scheme")
                .rect_color(invert);
            self.rectangles.push((bounds, color));
        }

        fn text(
            &mut self,
            bounds: Rect,
            _lpad: i32,
            text: &str,
            _invert: bool,
            _detail_height: i32,
        ) -> i32 {
            self.texts.push(text.to_string());
            bounds.x + bounds.w
        }
    }

    fn scheme() -> BarScheme {
        use crate::bar::color::Rgba;
        BarScheme {
            foreground: Rgba::new(1.0, 1.0, 1.0, 1.0),
            background: Rgba::rgb(0.0, 0.0, 0.0),
            detail: Rgba::new(0.5, 0.5, 0.5, 0.5),
        }
    }

    fn block(full_text: &str) -> I3Block {
        I3Block {
            full_text: full_text.to_string(),
            short_text: None,
            color: None,
            background: None,
            border: None,
            border_widths: Insets::default(),
            min_width: None,
            align: I3Align::Left,
            urgent: false,
            separator: true,
            separator_block_width: 9,
            name: None,
            instance: None,
            markup: None,
        }
    }

    #[test]
    fn uses_full_text_when_it_fits_and_short_text_when_needed() {
        let mut item = block("processor");
        item.short_text = Some("cpu".to_string());
        let items = vec![StatusItem::I3Block(item)];

        let mut wide = RecordingPainter::default();
        draw_status_items(Rect::new(0, 0, 200, 20), &items, scheme(), None, &mut wide);
        assert_eq!(wide.texts, ["processor"]);

        let mut narrow = RecordingPainter::default();
        draw_status_items(Rect::new(0, 0, 50, 20), &items, scheme(), None, &mut narrow);
        assert_eq!(narrow.texts, ["cpu"]);
    }

    #[test]
    fn separator_gap_is_not_part_of_click_target() {
        let mut first = block("a");
        first.separator = false;
        let items = vec![StatusItem::I3Block(first), StatusItem::I3Block(block("b"))];
        let mut painter = RecordingPainter::default();

        let output = draw_status_items(
            Rect::new(0, 0, 100, 20),
            &items,
            scheme(),
            None,
            &mut painter,
        );
        assert_eq!(output.click_targets.len(), 2);
        let first = output.click_targets[0].bounds;
        let second = output.click_targets[1].bounds;
        assert_eq!(second.x - (first.x + first.w), 9);
        assert_eq!(
            hit_test_i3_click_target(
                &output.click_targets,
                Point::new(first.x + first.w, first.y + 1),
            ),
            None
        );
    }

    #[test]
    fn hovered_block_gets_an_accent_without_recoloring_its_contents() {
        let items = vec![
            StatusItem::I3Block(block("cpu")),
            StatusItem::I3Block(block("memory")),
        ];
        let hover_color = Rgba::rgb(0.2, 0.8, 1.0);
        let mut painter = RecordingPainter::default();

        let output = draw_status_items(
            Rect::new(0, 0, 200, 20),
            &items,
            scheme(),
            Some(StatusBlockHover {
                block_index: 1,
                color: hover_color,
            }),
            &mut painter,
        );
        let hovered = output.click_targets[1].bounds;
        let indicator = Rect::new(
            hovered.x,
            hovered.bottom() - HOVER_INDICATOR_HEIGHT,
            hovered.w,
            HOVER_INDICATOR_HEIGHT,
        );

        assert!(painter.rectangles.contains(&(indicator, hover_color)));
        assert_eq!(
            painter
                .rectangles
                .iter()
                .filter(|(_, color)| *color == hover_color)
                .count(),
            1
        );
    }

    #[test]
    fn click_event_preserves_each_coordinate_space() {
        let block = block("cpu");
        let target = StatusClickTarget {
            bounds: Rect::new(80, 0, 40, 24),
            block_index: 0,
        };

        let event = make_i3_click_event(
            &block,
            target,
            1,
            StatusClickGeometry {
                root_position: Point::new(2000, 30),
                output_position: Point::new(80, 30),
                bar_position: Point::new(95, 10),
            },
            0,
        );

        assert_eq!((event.x, event.y), (2000, 30));
        assert_eq!((event.output_x, event.output_y), (80, 30));
        assert_eq!((event.relative_x, event.relative_y), (15, 10));
        assert_eq!((event.width, event.height), (40, 24));
    }

    #[test]
    fn empty_blocks_have_no_layout_or_click_target() {
        let items = vec![StatusItem::I3Block(block(""))];
        let mut painter = RecordingPainter::default();
        let output = draw_status_items(
            Rect::new(0, 0, 100, 20),
            &items,
            scheme(),
            None,
            &mut painter,
        );

        assert_eq!(output.bounds, Rect::default());
        assert!(output.click_targets.is_empty());
        assert!(painter.texts.is_empty());
    }

    #[test]
    fn min_width_is_the_complete_block_width() {
        let mut item = block("x");
        item.min_width = Some(I3MinWidth::Pixels(50));
        let items = vec![StatusItem::I3Block(item)];
        let mut painter = RecordingPainter::default();

        let output = draw_status_items(
            Rect::new(0, 0, 100, 20),
            &items,
            scheme(),
            None,
            &mut painter,
        );

        assert_eq!(output.click_targets[0].bounds.w, 50);
        assert_eq!(output.bounds.w, 52);
    }

    #[test]
    fn final_block_has_no_trailing_separator_gap() {
        let items = vec![StatusItem::I3Block(block("x"))];
        let mut painter = RecordingPainter::default();

        let output = draw_status_items(
            Rect::new(0, 0, 100, 20),
            &items,
            scheme(),
            None,
            &mut painter,
        );
        let block_width = output.click_targets[0].bounds.w;

        assert_eq!(output.bounds.w, block_width + 2);
    }

    #[test]
    fn empty_blocks_keep_protocol_block_indices() {
        let items = vec![
            StatusItem::I3Block(block("")),
            StatusItem::I3Block(block("visible")),
        ];
        let mut painter = RecordingPainter::default();

        let output = draw_status_items(
            Rect::new(0, 0, 120, 20),
            &items,
            scheme(),
            None,
            &mut painter,
        );

        assert_eq!(output.click_targets[0].block_index, 1);
    }

    #[test]
    fn measures_each_block_variant_only_once_per_layout() {
        let items = (0..4)
            .map(|index| {
                let mut item = block(&format!("processor-{index}"));
                item.short_text = Some(format!("p{index}"));
                item.min_width = Some(I3MinWidth::Text("processor-100".to_string()));
                StatusItem::I3Block(item)
            })
            .collect::<Vec<_>>();
        let mut painter = RecordingPainter::default();

        // Force every short-text candidate to be considered. Width selection
        // after measurement must remain arithmetic-only.
        draw_status_items(
            Rect::new(0, 0, 20, 20),
            &items,
            scheme(),
            None,
            &mut painter,
        );

        // One measurement each for full_text, short_text, and textual min_width.
        assert_eq!(painter.measurement_calls, items.len() * 3);
    }

    #[test]
    fn empty_short_text_can_hide_a_block_when_space_is_constrained() {
        let mut item = block("processor");
        item.short_text = Some(String::new());
        let items = vec![StatusItem::I3Block(item)];
        let mut painter = RecordingPainter::default();

        let output = draw_status_items(
            Rect::new(0, 0, 20, 20),
            &items,
            scheme(),
            None,
            &mut painter,
        );

        assert_eq!(output.bounds, Rect::default());
        assert!(output.click_targets.is_empty());
        assert!(painter.texts.is_empty());
    }
}
