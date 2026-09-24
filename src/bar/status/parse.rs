use super::model::{DEFAULT_SEPARATOR_BLOCK_WIDTH, RawI3Block};
use super::{I3Align, I3BarHeader, I3BarSignals, I3Block, I3MinWidth, StatusBlocks};
use crate::types::Insets;
use serde_json::Value;

pub(crate) fn parse_i3bar_json(text: &str) -> Option<StatusBlocks> {
    let mut json_str = text.trim();
    if let Some(rest) = json_str.strip_prefix(',') {
        json_str = rest.trim_start();
    }
    if let Some(rest) = json_str.strip_suffix(',') {
        json_str = rest.trim_end();
    }

    let raw_blocks: Vec<RawI3Block> = serde_json::from_str(json_str).ok()?;
    let blocks = raw_blocks.into_iter().map(|raw| {
        let align = match raw.align.as_deref() {
            Some("center") => I3Align::Center,
            Some("right") => I3Align::Right,
            _ => I3Align::Left,
        };

        let min_width = match raw.min_width {
            Some(Value::String(s)) => Some(I3MinWidth::Text(s)),
            Some(Value::Number(n)) => n
                .as_i64()
                .map(|v| I3MinWidth::Pixels(v.clamp(i32::MIN as i64, i32::MAX as i64) as i32)),
            _ => None,
        };

        let border = raw.border.filter(|c| c.starts_with('#'));
        let has_border = border.is_some();

        I3Block {
            full_text: raw.full_text,
            short_text: raw.short_text,
            color: raw.color.filter(|c| c.starts_with('#')),
            background: raw.background.filter(|c| c.starts_with('#')),
            border,
            border_widths: Insets::new(
                raw.border_top
                    .unwrap_or(if has_border { 1 } else { 0 })
                    .max(0),
                raw.border_right
                    .unwrap_or(if has_border { 1 } else { 0 })
                    .max(0),
                raw.border_bottom
                    .unwrap_or(if has_border { 1 } else { 0 })
                    .max(0),
                raw.border_left
                    .unwrap_or(if has_border { 1 } else { 0 })
                    .max(0),
            ),
            min_width,
            align,
            urgent: raw.urgent,
            separator: raw.separator,
            separator_block_width: raw
                .separator_block_width
                .unwrap_or(DEFAULT_SEPARATOR_BLOCK_WIDTH)
                .max(0),
            name: raw.name,
            instance: raw.instance,
            markup: raw.markup,
        }
    });
    Some(blocks.collect())
}

/// Parse an i3bar JSON frame, falling back to plain text.
pub(crate) fn parse_status(text: &str) -> StatusBlocks {
    parse_i3bar_json(text).unwrap_or_else(|| plain_text_status(text))
}

pub(crate) fn plain_text_status(text: &str) -> StatusBlocks {
    if text.is_empty() {
        return StatusBlocks::default();
    }
    StatusBlocks::from([I3Block {
        full_text: text.to_string(),
        separator: false,
        separator_block_width: 0,
        ..I3Block::default()
    }])
}

pub(crate) fn parse_i3bar_header(line: &str) -> Option<I3BarHeader> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    let obj = value.as_object()?;
    if obj.get("version").and_then(Value::as_i64) != Some(1) {
        return None;
    }

    let stop_signal = protocol_signal(obj.get("stop_signal"), libc::SIGSTOP, true);
    let suspension = (stop_signal != 0).then(|| I3BarSignals {
        stop: stop_signal,
        resume: protocol_signal(obj.get("cont_signal"), libc::SIGCONT, false),
    });

    Some(I3BarHeader {
        click_events: obj
            .get("click_events")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        suspension,
    })
}

fn protocol_signal(value: Option<&Value>, default: i32, allow_zero: bool) -> i32 {
    value
        .and_then(Value::as_i64)
        .and_then(|signal| i32::try_from(signal).ok())
        .filter(|&signal| signal > 0 || allow_zero && signal == 0)
        .unwrap_or(default)
}
