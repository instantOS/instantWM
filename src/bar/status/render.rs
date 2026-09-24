use super::TEXT_PADDING;
use super::{I3Align, I3Block, I3ClickEvent, I3MinWidth, StatusClickTarget};
use crate::bar::paint::{BarPainter, BarScheme, draw_hover_accent};
use crate::types::{Point, Rect, Rgba};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct StatusBlockHover {
    pub block_index: usize,
    pub color: Rgba,
}

pub(crate) struct StatusRenderOptions {
    pub base_scheme: BarScheme,
    pub separator_color: Rgba,
    pub hover: Option<StatusBlockHover>,
    pub edge_padding: i32,
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
struct MeasuredBlock {
    full: Option<BlockMetrics>,
    short: Option<BlockMetrics>,
    has_short: bool,
}

impl MeasuredBlock {
    fn metrics(self, use_short: bool) -> Option<BlockMetrics> {
        if use_short && self.has_short {
            self.short
        } else {
            self.full
        }
    }

    fn width(self, use_short: bool) -> i32 {
        self.metrics(use_short).map_or(0, |metrics| metrics.width)
    }
}

#[derive(Debug, Clone, Copy)]
struct LaidOutBlock {
    block_index: usize,
    bounds: Rect,
    text_bounds: Rect,
    text_lpad: i32,
    separator_bounds: Option<Rect>,
    use_short: bool,
}

#[derive(Debug, Default)]
struct StatusLayout {
    clip_bounds: Rect,
    blocks: Vec<LaidOutBlock>,
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
    if mask & crate::config::keybindings::MOD2 != 0 {
        modifiers.push("Mod2".to_string());
    }
    if mask & crate::config::keybindings::MOD3 != 0 {
        modifiers.push("Mod3".to_string());
    }
    if mask & crate::config::keybindings::MODKEY != 0 {
        modifiers.push("Mod4".to_string());
    }
    if mask & crate::config::keybindings::MOD5 != 0 {
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

/// Resolve a status-area click to the i3bar click event it reports.
pub(crate) fn i3_click_event(
    blocks: &[I3Block],
    click_targets: &[StatusClickTarget],
    geometry: StatusClickGeometry,
    button: u8,
    clean_state: u32,
) -> Option<I3ClickEvent> {
    let target = click_targets
        .iter()
        .copied()
        .find(|target| target.bounds.contains_point(geometry.bar_position))?;
    let block = blocks.get(target.block_index)?;
    Some(make_i3_click_event(
        block,
        target,
        button,
        geometry,
        clean_state,
    ))
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

fn measure_block_variant(
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

fn measure_blocks(blocks: &[I3Block], painter: &mut dyn BarPainter) -> Vec<MeasuredBlock> {
    blocks
        .iter()
        .map(|block| {
            let min_width = match &block.min_width {
                Some(I3MinWidth::Text(text)) => painter.text_width(text).max(0),
                Some(I3MinWidth::Pixels(pixels)) => (*pixels).max(0),
                None => 0,
            };
            MeasuredBlock {
                full: measure_block_variant(block, block.full_text.as_str(), min_width, painter),
                short: block
                    .short_text
                    .as_deref()
                    .and_then(|text| measure_block_variant(block, text, min_width, painter)),
                has_short: block.short_text.is_some(),
            }
        })
        .collect()
}

fn measured_width(blocks: &[I3Block], measured: &[MeasuredBlock], choices: &[bool]) -> i32 {
    let mut width = 0i32;
    let mut has_later_visible_block = false;

    for (index, block) in blocks.iter().enumerate().rev() {
        let block_width = measured[index].width(choices[index]);
        if block_width <= 0 {
            continue;
        }
        if has_later_visible_block {
            width = width.saturating_add(block.separator_block_width.max(0));
        }
        width = width.saturating_add(block_width);
        has_later_visible_block = true;
    }

    width
}

fn choose_short_texts(
    blocks: &[I3Block],
    measured: &[MeasuredBlock],
    max_content_width: i32,
) -> Vec<bool> {
    let mut choices = vec![false; blocks.len()];
    let mut width = measured_width(blocks, measured, &choices);

    for (index, block) in blocks.iter().enumerate() {
        if width <= max_content_width {
            break;
        }
        if block.short_text.is_none() {
            continue;
        }

        let previous_choices = choices.clone();
        choices[index] = true;
        if let Some(name) = block.name.as_deref() {
            for (other_index, other) in blocks.iter().enumerate() {
                if other.name.as_deref() == Some(name) && other.short_text.is_some() {
                    choices[other_index] = true;
                }
            }
        }
        let shortened_width = measured_width(blocks, measured, &choices);
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
    blocks: &[I3Block],
    edge_padding: i32,
    painter: &mut dyn BarPainter,
) -> StatusLayout {
    let available_bounds = Rect::new(
        available_bounds.x,
        available_bounds.y,
        available_bounds.w.max(0),
        available_bounds.h.max(0),
    );
    let edge_padding = edge_padding.max(0);
    let measured = measure_blocks(blocks, painter);
    let choices = choose_short_texts(
        blocks,
        &measured,
        available_bounds
            .w
            .saturating_sub(edge_padding.saturating_mul(2))
            .max(0),
    );
    let total_width = measured_width(blocks, &measured, &choices);
    if total_width <= 0 || available_bounds.w <= 0 || available_bounds.h <= 0 {
        return StatusLayout::default();
    }

    let background_width = total_width.saturating_add(edge_padding.saturating_mul(2));
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
    let mut laid_out = Vec::with_capacity(blocks.len());
    let mut x = background_bounds.x.saturating_add(edge_padding);
    let last_visible_block =
        (0..blocks.len()).rfind(|&index| measured[index].width(choices[index]) > 0);

    for (block_index, (block, measured_block)) in blocks.iter().zip(&measured).enumerate() {
        let use_short = choices[block_index];
        let Some(metrics) = measured_block.metrics(use_short) else {
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
            if Some(block_index) != last_visible_block && block.separator_block_width > 0 {
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

        laid_out.push(LaidOutBlock {
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
    }

    StatusLayout {
        clip_bounds,
        blocks: laid_out,
    }
}

/// Paint the status line right-aligned inside `available_bounds`.
///
/// When the line is wider than the available space, blocks at its left edge
/// are clipped, like i3bar truncating the statusline.
pub(crate) fn draw_status_blocks(
    available_bounds: Rect,
    blocks: &[I3Block],
    options: StatusRenderOptions,
    painter: &mut dyn BarPainter,
) -> StatusRenderOutput {
    let StatusRenderOptions {
        base_scheme,
        separator_color,
        hover,
        edge_padding,
    } = options;
    let layout = measure_layout(available_bounds, blocks, edge_padding, painter);
    if layout.clip_bounds.w <= 0 || layout.clip_bounds.h <= 0 {
        return StatusRenderOutput::default();
    }

    painter.set_scheme(base_scheme.clone());
    painter.rect(layout.clip_bounds, true);

    let mut click_targets = Vec::new();
    for laid_out in layout.blocks {
        let Some(visible) = laid_out.bounds.intersection(&layout.clip_bounds) else {
            continue;
        };
        let block = &blocks[laid_out.block_index];
        draw_i3_block(
            painter,
            laid_out,
            visible,
            block_text(block, laid_out.use_short),
            block,
            &base_scheme,
        );
        if let Some(hover) = hover.filter(|hover| hover.block_index == laid_out.block_index) {
            draw_hover_accent(painter, visible, hover.color);
        }
        click_targets.push(StatusClickTarget {
            bounds: visible,
            block_index: laid_out.block_index,
        });

        if let Some(separator_bounds) = laid_out.separator_bounds {
            draw_separator(
                painter,
                separator_bounds,
                block.separator,
                separator_color,
                &base_scheme,
            );
        }
    }

    StatusRenderOutput {
        bounds: layout.clip_bounds,
        click_targets,
    }
}

fn draw_i3_block(
    painter: &mut dyn BarPainter,
    laid_out: LaidOutBlock,
    visible: Rect,
    text: &str,
    block: &I3Block,
    base_scheme: &BarScheme,
) {
    let bounds = laid_out.bounds;
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
    painter.rect(visible, true);

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
    let top = border.top.min(bounds.h);
    let bottom = border.bottom.min(bounds.h);
    let left = border.left.min(bounds.w);
    let right = border.right.min(bounds.w);
    for edge in [
        Rect::new(bounds.x, bounds.y, bounds.w, top),
        Rect::new(bounds.x, bounds.bottom() - bottom, bounds.w, bottom),
        Rect::new(bounds.x, bounds.y, left, bounds.h),
        Rect::new(bounds.right() - right, bounds.y, right, bounds.h),
    ] {
        if let Some(edge) = edge.intersection(&visible) {
            painter.rect(edge, false);
        }
    }

    if let Some(text_bounds) = laid_out.text_bounds.intersection(&visible) {
        let lpad = (laid_out.text_bounds.x + laid_out.text_lpad - text_bounds.x).max(0);
        painter.set_scheme(block_scheme);
        painter.text(text_bounds, lpad, text, false, 0);
    }
}

fn draw_separator(
    painter: &mut dyn BarPainter,
    bounds: Rect,
    draw_line: bool,
    separator_color: Rgba,
    base_scheme: &BarScheme,
) {
    painter.set_scheme(base_scheme.clone());
    painter.rect(bounds, true);
    if !draw_line || bounds.w <= 0 || bounds.h <= 0 {
        return;
    }

    let line_height = (bounds.h - 8).max(1).min(bounds.h);
    let line_y = bounds.y + (bounds.h - line_height) / 2;
    let line_x = bounds.x + bounds.w / 2;
    painter.set_scheme(BarScheme {
        foreground: separator_color,
        background: base_scheme.background,
        detail: base_scheme.detail,
    });
    painter.rect(Rect::new(line_x, line_y, 1, line_height), false);
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bar::paint::HOVER_INDICATOR_HEIGHT;
    use crate::types::{Insets, Size};

    #[derive(Default)]
    struct RecordingPainter {
        texts: Vec<String>,
        text_bounds: Vec<Rect>,
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

        fn rect(&mut self, bounds: Rect, invert: bool) {
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
            self.text_bounds.push(bounds);
            bounds.x + bounds.w
        }

        fn blit_rgba(&mut self, destination: Rect, source_size: Size, src_rgba: &[u8]) {
            assert!(
                src_rgba.len() >= (source_size.w as usize) * (source_size.h as usize) * 4,
                "blit_rgba requires enough source pixels"
            );
            self.rectangles
                .push((destination, Rgba::rgb(1.0, 1.0, 1.0)));
        }
    }

    fn scheme() -> BarScheme {
        use crate::types::Rgba;
        BarScheme {
            foreground: Rgba::new(1.0, 1.0, 1.0, 1.0),
            background: Rgba::rgb(0.0, 0.0, 0.0),
            detail: Rgba::new(0.5, 0.5, 0.5, 0.5),
        }
    }

    fn render_options() -> StatusRenderOptions {
        StatusRenderOptions {
            base_scheme: scheme(),
            separator_color: Rgba::new(0.25, 0.25, 0.25, 1.0),
            hover: None,
            edge_padding: 1,
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
        let items = vec![item];

        let mut wide = RecordingPainter::default();
        draw_status_blocks(
            Rect::new(0, 0, 200, 20),
            &items,
            render_options(),
            &mut wide,
        );
        assert_eq!(wide.texts, ["processor"]);

        let mut narrow = RecordingPainter::default();
        draw_status_blocks(
            Rect::new(0, 0, 50, 20),
            &items,
            render_options(),
            &mut narrow,
        );
        assert_eq!(narrow.texts, ["cpu"]);
    }

    #[test]
    fn plain_status_reserves_edge_padding_on_both_sides() {
        let items = super::super::parse::plain_text_status("cpu");
        let mut painter = RecordingPainter::default();

        let output = draw_status_blocks(
            Rect::new(0, 0, 100, 20),
            &items,
            StatusRenderOptions {
                edge_padding: 8,
                ..render_options()
            },
            &mut painter,
        );

        assert_eq!(output.bounds, Rect::new(54, 0, 46, 20));
        assert_eq!(painter.text_bounds, [Rect::new(62, 0, 30, 20)]);
    }

    #[test]
    fn overflowing_status_is_clipped_at_its_left_edge() {
        let items = super::super::parse::plain_text_status("a long status line");
        let mut painter = RecordingPainter::default();

        let output = draw_status_blocks(
            Rect::new(40, 0, 60, 20),
            &items,
            render_options(),
            &mut painter,
        );

        assert_eq!(output.bounds, Rect::new(40, 0, 60, 20));
        assert_eq!(painter.texts, ["a long status line"]);
        assert_eq!(painter.text_bounds, [Rect::new(40, 0, 59, 20)]);
        assert!(
            painter
                .rectangles
                .iter()
                .all(|(bounds, _)| bounds.x >= 40 && bounds.right() <= 100),
            "nothing may be painted outside the status area"
        );
    }

    #[test]
    fn separator_gap_is_not_part_of_click_target() {
        let mut first = block("a");
        first.separator = false;
        let items = vec![first, block("b")];
        let mut painter = RecordingPainter::default();

        let output = draw_status_blocks(
            Rect::new(0, 0, 100, 20),
            &items,
            render_options(),
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
    fn separator_uses_its_dedicated_color() {
        let separator_color = Rgba::rgb(0.25, 0.3, 0.35);
        let mut painter = RecordingPainter::default();

        draw_separator(
            &mut painter,
            Rect::new(10, 0, 9, 20),
            true,
            separator_color,
            &scheme(),
        );

        assert_eq!(
            painter.rectangles.last(),
            Some(&(Rect::new(14, 4, 1, 12), separator_color))
        );
    }

    #[test]
    fn hovered_block_gets_an_accent_without_recoloring_its_contents() {
        let items = vec![block("cpu"), block("memory")];
        let hover_color = Rgba::rgb(0.2, 0.8, 1.0);
        let mut painter = RecordingPainter::default();

        let output = draw_status_blocks(
            Rect::new(0, 0, 200, 20),
            &items,
            StatusRenderOptions {
                hover: Some(StatusBlockHover {
                    block_index: 1,
                    color: hover_color,
                }),
                ..render_options()
            },
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
        let items = vec![block("")];
        let mut painter = RecordingPainter::default();
        let output = draw_status_blocks(
            Rect::new(0, 0, 100, 20),
            &items,
            render_options(),
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
        let items = vec![item];
        let mut painter = RecordingPainter::default();

        let output = draw_status_blocks(
            Rect::new(0, 0, 100, 20),
            &items,
            render_options(),
            &mut painter,
        );

        assert_eq!(output.click_targets[0].bounds.w, 50);
        assert_eq!(output.bounds.w, 52);
    }

    #[test]
    fn final_block_has_no_trailing_separator_gap() {
        let items = vec![block("x")];
        let mut painter = RecordingPainter::default();

        let output = draw_status_blocks(
            Rect::new(0, 0, 100, 20),
            &items,
            render_options(),
            &mut painter,
        );
        let block_width = output.click_targets[0].bounds.w;

        assert_eq!(output.bounds.w, block_width + 2);
    }

    #[test]
    fn empty_blocks_keep_protocol_block_indices() {
        let items = vec![block(""), block("visible")];
        let mut painter = RecordingPainter::default();

        let output = draw_status_blocks(
            Rect::new(0, 0, 120, 20),
            &items,
            render_options(),
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
                item
            })
            .collect::<Vec<_>>();
        let mut painter = RecordingPainter::default();

        // Force every short-text candidate to be considered. Width selection
        // after measurement must remain arithmetic-only.
        draw_status_blocks(
            Rect::new(0, 0, 20, 20),
            &items,
            render_options(),
            &mut painter,
        );

        // One measurement each for full_text, short_text, and textual min_width.
        assert_eq!(painter.measurement_calls, items.len() * 3);
    }

    #[test]
    fn empty_short_text_can_hide_a_block_when_space_is_constrained() {
        let mut item = block("processor");
        item.short_text = Some(String::new());
        let items = vec![item];
        let mut painter = RecordingPainter::default();

        let output = draw_status_blocks(
            Rect::new(0, 0, 20, 20),
            &items,
            render_options(),
            &mut painter,
        );

        assert_eq!(output.bounds, Rect::default());
        assert!(output.click_targets.is_empty());
        assert!(painter.texts.is_empty());
    }
}
