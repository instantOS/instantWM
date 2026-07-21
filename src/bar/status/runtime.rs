use super::I3ClickEvent;
use std::io;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Mutex, OnceLock};

#[derive(Debug)]
struct I3ClickRuntime {
    sender: Sender<I3ClickEvent>,
    receiver: Mutex<Receiver<I3ClickEvent>>,
}

#[derive(Debug)]
struct InternalStatusRuntime {
    sender: Sender<StatusUpdate>,
    receiver: Mutex<Receiver<StatusUpdate>>,
    ping: Mutex<Option<calloop::ping::Ping>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusUpdate {
    text: String,
    click_events: bool,
}

static I3BAR_CLICK_RUNTIME: OnceLock<I3ClickRuntime> = OnceLock::new();
static INTERNAL_STATUS_RUNTIME: OnceLock<InternalStatusRuntime> = OnceLock::new();

fn i3bar_click_runtime() -> &'static I3ClickRuntime {
    I3BAR_CLICK_RUNTIME.get_or_init(|| {
        let (sender, receiver) = mpsc::channel();
        I3ClickRuntime {
            sender,
            receiver: Mutex::new(receiver),
        }
    })
}

fn internal_status_runtime() -> &'static InternalStatusRuntime {
    INTERNAL_STATUS_RUNTIME.get_or_init(|| {
        let (sender, receiver) = mpsc::channel();
        InternalStatusRuntime {
            sender,
            receiver: Mutex::new(receiver),
            ping: Mutex::new(None),
        }
    })
}

pub(crate) fn set_internal_status_ping(ping: calloop::ping::Ping) {
    let runtime = internal_status_runtime();
    if let Ok(mut slot) = runtime.ping.lock() {
        *slot = Some(ping);
    }
}

pub(super) fn send_status_update(text: &str, click_events: bool) {
    let runtime = internal_status_runtime();
    let _ = runtime.sender.send(StatusUpdate {
        text: text.to_string(),
        click_events,
    });
    if let Ok(guard) = runtime.ping.lock()
        && let Some(ping) = guard.as_ref()
    {
        ping.ping();
    }
}

fn stop_default_source() {
    super::command::stop_default_source();
}

pub(crate) fn apply_status_update(wm: &mut crate::wm::Wm, text: String) {
    apply_status_update_with_capabilities(
        wm,
        StatusUpdate {
            text,
            click_events: false,
        },
    );
}

fn apply_status_update_with_capabilities(wm: &mut crate::wm::Wm, update: StatusUpdate) {
    if !update_bar_status(&mut wm.bar, update) {
        return;
    }

    stop_default_source();
}

fn update_bar_status(bar: &mut crate::bar::BarState, update: StatusUpdate) -> bool {
    if bar.runtime.status_text == update.text
        && bar.runtime.status_click_events == update.click_events
    {
        return false;
    }
    bar.prepare_status_for_render(&update.text);
    bar.runtime.status_text = update.text;
    bar.runtime.status_click_events = update.click_events;
    bar.mark_dirty();
    true
}

pub(crate) fn drain_internal_status_updates(wm: &mut crate::wm::Wm) -> bool {
    let runtime = internal_status_runtime();
    let Ok(receiver) = runtime.receiver.lock() else {
        return false;
    };

    let mut latest = None;
    loop {
        match receiver.try_recv() {
            Ok(update) => latest = Some(update),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
        }
    }

    let Some(update) = latest else {
        return false;
    };

    apply_status_update_with_capabilities(wm, update);
    true
}

pub(super) fn enqueue_i3bar_click_event(event: I3ClickEvent) {
    let _ = i3bar_click_runtime().sender.send(event);
}

fn try_recv_i3bar_click_event() -> Option<I3ClickEvent> {
    let receiver = i3bar_click_runtime().receiver.lock().ok()?;
    match receiver.try_recv() {
        Ok(event) => Some(event),
        Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
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

pub(crate) fn flush_i3bar_click_events<W: io::Write>(
    writer: &mut W,
    first_event: &mut bool,
) -> io::Result<()> {
    while let Some(event) = try_recv_i3bar_click_event() {
        write_i3bar_click_event(&mut *writer, &event, first_event)?;
    }
    writer.flush()
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
        let text = r#"[{"full_text":"cpu"}]"#.to_string();

        assert!(update_bar_status(
            &mut bar,
            StatusUpdate {
                text: text.clone(),
                click_events: true,
            },
        ));
        let first_seq = bar.update_seq();
        assert!(bar.runtime.status_click_events);

        assert!(update_bar_status(
            &mut bar,
            StatusUpdate {
                text,
                click_events: false,
            },
        ));
        assert!(!bar.runtime.status_click_events);
        assert_ne!(bar.update_seq(), first_seq);
    }
}
