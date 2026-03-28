use super::model::{DEFAULT_SEPARATOR_BLOCK_WIDTH, RawI3Block};
use super::{I3Align, I3BarHeader, I3Block, I3MinWidth, I3StatusLine, ParsedStatus, StatusItem};
use serde_json::Value;

pub(crate) fn parse_i3bar_json(bytes: &[u8]) -> Option<ParsedStatus> {
    let mut json_str = std::str::from_utf8(bytes).ok()?.trim();
    if let Some(rest) = json_str.strip_prefix(',') {
        json_str = rest.trim_start();
    }
    if let Some(rest) = json_str.strip_suffix(',') {
        json_str = rest.trim_end();
    }

    let raw_blocks: Vec<RawI3Block> = serde_json::from_str(json_str).ok()?;
    let mut blocks = Vec::with_capacity(raw_blocks.len());
    let mut items = Vec::with_capacity(raw_blocks.len());

    for raw in raw_blocks {
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

        let block = I3Block {
            full_text: raw.full_text,
            short_text: raw.short_text,
            color: raw.color.filter(|c| c.starts_with('#')),
            background: raw.background.filter(|c| c.starts_with('#')),
            border,
            border_top: raw
                .border_top
                .unwrap_or(if has_border { 1 } else { 0 })
                .max(0),
            border_right: raw
                .border_right
                .unwrap_or(if has_border { 1 } else { 0 })
                .max(0),
            border_bottom: raw
                .border_bottom
                .unwrap_or(if has_border { 1 } else { 0 })
                .max(0),
            border_left: raw
                .border_left
                .unwrap_or(if has_border { 1 } else { 0 })
                .max(0),
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
        };

        items.push(StatusItem::I3Block(block.clone()));
        blocks.push(block);
    }

    Some(ParsedStatus {
        items,
        i3bar: Some(I3StatusLine { blocks }),
    })
}

pub(crate) fn parse_status(bytes: &[u8]) -> ParsedStatus {
    if let Some(parsed) = parse_i3bar_json(bytes) {
        return parsed;
    }

    parse_status_fallback(std::str::from_utf8(bytes).unwrap_or(""))
}

pub(crate) fn parse_status_fallback(text: &str) -> ParsedStatus {
    if text.is_empty() {
        return ParsedStatus::default();
    }

    ParsedStatus {
        items: vec![StatusItem::Text(text.to_string())],
        i3bar: None,
    }
}

pub(crate) fn parse_i3bar_header(line: &str) -> Option<I3BarHeader> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    let obj = value.as_object()?;

    Some(I3BarHeader {
        version: obj
            .get("version")
            .and_then(Value::as_i64)
            .map(|v| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32),
        click_events: obj
            .get("click_events")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        stop_signal: obj
            .get("stop_signal")
            .and_then(Value::as_i64)
            .map(|v| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32),
        cont_signal: obj
            .get("cont_signal")
            .and_then(Value::as_i64)
            .map(|v| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32),
    })
}
