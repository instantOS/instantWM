use super::I3ClickEvent;
use super::command::StatusUpdate;
use std::io;

impl crate::bar::BarState {
    /// Apply the newest frame of the active status source.
    pub(crate) fn drain_status_updates(&mut self) -> bool {
        let Some(update) = self.status_sources.take_latest_update() else {
            return false;
        };
        self.apply_status_update(update);
        true
    }

    /// Replace the status with externally supplied text (IPC).
    pub(crate) fn set_status_text(&mut self, text: &str) {
        self.apply_status_update(StatusUpdate {
            blocks: super::parse_status(text),
            click_events: false,
        });
    }

    fn apply_status_update(&mut self, update: StatusUpdate) -> bool {
        let runtime = &mut self.runtime;
        if *runtime.status == *update.blocks && runtime.status_click_events == update.click_events {
            return false;
        }
        runtime.status = update.blocks;
        runtime.status_click_events = update.click_events;
        self.mark_dirty();
        self.status_sources.stop_default_source();
        true
    }
}

pub(crate) fn write_i3bar_click_event<W: io::Write>(
    mut writer: W,
    event: &I3ClickEvent,
    first_event: &mut bool,
) -> io::Result<()> {
    if *first_event {
        writer.write_all(b"[\n")?;
        *first_event = false;
    } else {
        writer.write_all(b",\n")?;
    }

    serde_json::to_writer(&mut writer, event).map_err(io::Error::other)?;
    writer.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click_event() -> I3ClickEvent {
        I3ClickEvent {
            name: Some("cpu".to_string()),
            instance: None,
            button: 1,
            x: 100,
            y: 20,
            relative_x: 4,
            relative_y: 5,
            output_x: 100,
            output_y: 20,
            width: 30,
            height: 24,
            modifiers: Vec::new(),
        }
    }

    #[test]
    fn click_stream_starts_with_an_array_not_a_status_header() {
        let mut output = Vec::new();
        let mut first = true;

        write_i3bar_click_event(&mut output, &click_event(), &mut first).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("[\n{"));
        assert!(!output.contains("\"version\""));
        assert!(output.contains("\"output_x\":100"));
    }

    #[test]
    fn later_clicks_are_comma_separated() {
        let mut output = Vec::new();
        let mut first = true;

        write_i3bar_click_event(&mut output, &click_event(), &mut first).unwrap();
        write_i3bar_click_event(&mut output, &click_event(), &mut first).unwrap();

        assert!(String::from_utf8(output).unwrap().contains("\n,\n{"));
    }

    #[test]
    fn status_update_preserves_and_invalidates_click_capability() {
        let mut bar = crate::bar::BarState::default();
        let blocks = super::super::parse_status(r#"[{"full_text":"cpu"}]"#);

        assert!(bar.apply_status_update(StatusUpdate {
            blocks: blocks.clone(),
            click_events: true,
        }));
        let first_seq = bar.update_seq();
        assert!(bar.runtime.status_click_events);
        assert!(!bar.apply_status_update(StatusUpdate {
            blocks: blocks.clone(),
            click_events: true,
        }));
        assert_eq!(bar.update_seq(), first_seq);

        assert!(bar.apply_status_update(StatusUpdate {
            blocks,
            click_events: false,
        }));
        assert!(!bar.runtime.status_click_events);
        assert_ne!(bar.update_seq(), first_seq);
    }

    #[test]
    fn status_text_is_parsed_once_into_blocks() {
        let mut bar = crate::bar::BarState::default();

        bar.set_status_text(r#"[{"full_text":"cpu","name":"cpu"}]"#);
        assert_eq!(bar.runtime.status.len(), 1);
        assert_eq!(bar.runtime.status[0].name.as_deref(), Some("cpu"));

        bar.set_status_text("plain text");
        assert_eq!(bar.runtime.status[0].full_text, "plain text");
        assert!(!bar.runtime.status[0].separator);
        assert_eq!(bar.runtime.status[0].separator_block_width, 0);
    }
}
