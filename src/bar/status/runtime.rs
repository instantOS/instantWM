use super::I3ClickEvent;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Mutex, OnceLock};

#[derive(Debug)]
struct I3ClickRuntime {
    sender: Sender<I3ClickEvent>,
    receiver: Mutex<Receiver<I3ClickEvent>>,
}

#[derive(Debug)]
struct InternalStatusRuntime {
    sender: Sender<String>,
    receiver: Mutex<Receiver<String>>,
    ping: Mutex<Option<calloop::ping::Ping>>,
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

pub(super) fn send_status_update(text: &str) {
    let runtime = internal_status_runtime();
    let _ = runtime.sender.send(text.to_string());
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
    if wm.g.bar_runtime.status_text == text {
        return;
    }

    stop_default_source();

    wm.bar.prepare_status_for_render(&text);
    wm.g.bar_runtime.status_text = text;
    wm.bar.mark_dirty();
}

pub(crate) fn drain_internal_status_updates(wm: &mut crate::wm::Wm) -> bool {
    let runtime = internal_status_runtime();
    let Ok(receiver) = runtime.receiver.lock() else {
        return false;
    };

    let mut latest = None;
    loop {
        match receiver.try_recv() {
            Ok(text) => latest = Some(text),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
        }
    }

    let Some(text) = latest else {
        return false;
    };

    apply_status_update(wm, text);
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

pub(crate) fn write_i3bar_click_event<W: std::io::Write>(
    mut writer: W,
    event: &I3ClickEvent,
    first_event: &mut bool,
) -> std::io::Result<()> {
    if *first_event {
        writer.write_all(b"{\"version\":1,\"click_events\":true}\n[\n")?;
        *first_event = false;
    } else {
        writer.write_all(b",\n")?;
    }

    serde_json::to_writer(&mut writer, event).map_err(std::io::Error::other)?;
    writer.write_all(b"\n")
}

pub(crate) fn flush_i3bar_click_events<W: std::io::Write>(
    writer: &mut W,
    first_event: &mut bool,
) -> std::io::Result<()> {
    while let Some(event) = try_recv_i3bar_click_event() {
        write_i3bar_click_event(&mut *writer, &event, first_event)?;
    }
    writer.flush()
}
