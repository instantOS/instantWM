#![allow(clippy::large_enum_variant)]

mod command;
mod model;
mod parse;
mod render;
mod runtime;

pub(crate) use command::{spawn_default_status, spawn_status_command};
pub(crate) use model::{
    CUSTOM_STATUS_RECEIVED, I3Align, I3BarHeader, I3Block, I3ClickEvent, I3MinWidth, I3StatusLine,
    ParsedStatus, StatusClickTarget, StatusItem, StatusParseResult, TEXT_PADDING,
};
pub(crate) use parse::{parse_i3bar_header, parse_i3bar_json, parse_status, parse_status_fallback};
pub(crate) use render::{draw_status_items, emit_i3bar_status_click};
pub(crate) use runtime::{
    apply_status_update, drain_internal_status_updates, flush_i3bar_click_events,
    request_status_parse, set_internal_status_ping, try_recv_status_parse_result,
};

#[cfg(test)]
mod tests {
    use super::{I3Align, StatusItem, parse_i3bar_header, parse_i3bar_json, parse_status};

    #[test]
    fn parses_i3bar_frame_with_leading_comma() {
        let parsed = parse_i3bar_json(br##",[{"full_text":"cpu","color":"#ffffff"}]"##).unwrap();

        assert_eq!(parsed.items.len(), 1);
        let Some(StatusItem::I3Block(block)) = parsed.items.first() else {
            panic!("expected i3 block");
        };
        assert_eq!(block.full_text, "cpu");
        assert_eq!(block.color.as_deref(), Some("#ffffff"));
        assert_eq!(block.align, I3Align::Left);
    }

    #[test]
    fn parses_i3bar_frame_with_trailing_comma() {
        let parsed = parse_i3bar_json(br#"[{"full_text":"mem","separator":false}],"#).unwrap();

        assert_eq!(parsed.items.len(), 1);
        let Some(StatusItem::I3Block(block)) = parsed.items.first() else {
            panic!("expected i3 block");
        };
        assert_eq!(block.full_text, "mem");
        assert!(!block.separator);
    }

    #[test]
    fn parse_status_keeps_plain_text_fallback_for_non_json() {
        let parsed = parse_status(b"plain text");

        assert_eq!(parsed.items.len(), 1);
        let Some(StatusItem::Text(text)) = parsed.items.first() else {
            panic!("expected plain text item");
        };
        assert_eq!(text, "plain text");
        assert!(parsed.i3bar.is_none());
    }

    #[test]
    fn parses_i3bar_header_with_click_events() {
        let header =
            parse_i3bar_header(r#"{"version":1,"click_events":true,"stop_signal":19}"#).unwrap();

        assert_eq!(header.version, Some(1));
        assert!(header.click_events);
        assert_eq!(header.stop_signal, Some(19));
    }
}
