#![allow(clippy::large_enum_variant)]

mod command;
mod model;
mod parse;
mod render;
mod runtime;

pub(crate) use command::{StatusSources, sync_visibility};
pub(crate) use model::{
    I3Align, I3BarHeader, I3BarSignals, I3Block, I3ClickEvent, I3MinWidth, StatusBlocks,
    StatusClickTarget, TEXT_PADDING,
};
pub(crate) use parse::{parse_i3bar_header, parse_status, plain_text_status};
pub(crate) use render::{
    StatusBlockHover, StatusClickGeometry, StatusRenderOptions, StatusRenderOutput,
    draw_status_blocks, hit_test_i3_click_target, i3_click_event,
};

#[cfg(test)]
mod tests {
    use super::parse::parse_i3bar_json;
    use super::{I3Align, parse_i3bar_header, parse_status};

    #[test]
    fn parses_i3bar_frame_with_leading_comma() {
        let parsed = parse_i3bar_json(r##",[{"full_text":"cpu","color":"#ffffff"}]"##).unwrap();

        assert_eq!(parsed.len(), 1);
        let block = &parsed[0];
        assert_eq!(block.full_text, "cpu");
        assert_eq!(block.color.as_deref(), Some("#ffffff"));
        assert_eq!(block.align, I3Align::Left);
    }

    #[test]
    fn parses_i3bar_frame_with_trailing_comma() {
        let parsed = parse_i3bar_json(r#"[{"full_text":"mem","separator":false}],"#).unwrap();

        assert_eq!(parsed.len(), 1);
        let block = &parsed[0];
        assert_eq!(block.full_text, "mem");
        assert!(!block.separator);
    }

    #[test]
    fn parse_status_keeps_plain_text_fallback_for_non_json() {
        let parsed = parse_status("plain text");

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].full_text, "plain text");
        assert!(!parsed[0].separator);
        assert_eq!(parsed[0].separator_block_width, 0);
        assert!(parse_status("").is_empty());
    }

    #[test]
    fn parses_i3bar_header_with_click_events() {
        let header =
            parse_i3bar_header(r#"{"version":1,"click_events":true,"stop_signal":19}"#).unwrap();

        assert!(header.click_events);
        assert_eq!(
            header.suspension,
            Some(super::I3BarSignals {
                stop: 19,
                resume: libc::SIGCONT,
            })
        );
    }

    #[test]
    fn i3bar_header_uses_default_suspension_signals() {
        let header = parse_i3bar_header(r#"{"version":1}"#).unwrap();

        assert_eq!(
            header.suspension,
            Some(super::I3BarSignals {
                stop: libc::SIGSTOP,
                resume: libc::SIGCONT,
            })
        );
    }

    #[test]
    fn i3bar_header_can_disable_suspension() {
        let header =
            parse_i3bar_header(r#"{"version":1,"stop_signal":0,"cont_signal":12}"#).unwrap();

        assert_eq!(header.suspension, None);
    }

    #[test]
    fn i3bar_header_accepts_custom_suspension_signals() {
        let header =
            parse_i3bar_header(r#"{"version":1,"stop_signal":10,"cont_signal":12}"#).unwrap();

        assert_eq!(
            header.suspension,
            Some(super::I3BarSignals {
                stop: 10,
                resume: 12,
            })
        );
    }

    #[test]
    fn i3bar_header_falls_back_from_invalid_signal_values() {
        let header =
            parse_i3bar_header(r#"{"version":1,"stop_signal":-1,"cont_signal":0}"#).unwrap();

        assert_eq!(
            header.suspension,
            Some(super::I3BarSignals {
                stop: libc::SIGSTOP,
                resume: libc::SIGCONT,
            })
        );
    }

    #[test]
    fn rejects_missing_or_unsupported_i3bar_versions() {
        assert!(parse_i3bar_header(r#"{"click_events":true}"#).is_none());
        assert!(parse_i3bar_header(r#"{"version":2,"click_events":true}"#).is_none());
    }

    #[test]
    fn groups_normalized_border_widths() {
        let parsed = parse_i3bar_json(
            r##"[{"full_text":"cpu","border":"#ffffff","border_top":2,"border_left":3}]"##,
        )
        .unwrap();
        let block = &parsed[0];

        assert_eq!(block.border_widths.top, 2);
        assert_eq!(block.border_widths.right, 1);
        assert_eq!(block.border_widths.bottom, 1);
        assert_eq!(block.border_widths.left, 3);
    }
}
