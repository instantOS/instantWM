use super::{CUSTOM_STATUS_RECEIVED, I3ClickEvent, StatusParseResult, parse_status};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

#[derive(Debug, Clone)]
struct PendingStatusParse {
    seq: u64,
    text: String,
}

#[derive(Debug)]
struct I3ClickRuntime {
    sender: Sender<I3ClickEvent>,
    receiver: Mutex<Receiver<I3ClickEvent>>,
}

#[derive(Debug)]
struct StatusParseShared {
    pending: Mutex<Option<PendingStatusParse>>,
    wake: Condvar,
    results_tx: Sender<StatusParseResult>,
}

#[derive(Debug)]
struct StatusParseRuntime {
    shared: Arc<StatusParseShared>,
    results_rx: Mutex<Receiver<StatusParseResult>>,
    next_seq: AtomicU64,
}

#[derive(Debug)]
struct InternalStatusRuntime {
    sender: Sender<String>,
    receiver: Mutex<Receiver<String>>,
    ping: Mutex<Option<calloop::ping::Ping>>,
}

static I3BAR_CLICK_RUNTIME: OnceLock<I3ClickRuntime> = OnceLock::new();
static STATUS_PARSE_RUNTIME: OnceLock<StatusParseRuntime> = OnceLock::new();
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

fn status_parse_runtime() -> &'static StatusParseRuntime {
    STATUS_PARSE_RUNTIME.get_or_init(|| {
        let (results_tx, results_rx) = mpsc::channel();
        let shared = Arc::new(StatusParseShared {
            pending: Mutex::new(None),
            wake: Condvar::new(),
            results_tx,
        });

        let worker_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("instantwm-status-parse".to_string())
            .spawn(move || {
                loop {
                    let pending = {
                        let mut guard = worker_shared.pending.lock().unwrap();
                        loop {
                            if let Some(pending) = guard.take() {
                                break pending;
                            }
                            guard = worker_shared.wake.wait(guard).unwrap();
                        }
                    };

                    let parsed = parse_status(pending.text.as_bytes());
                    let _ = worker_shared.results_tx.send(StatusParseResult {
                        seq: pending.seq,
                        text: pending.text,
                        parsed,
                    });
                }
            })
            .expect("failed to spawn status parser thread");

        StatusParseRuntime {
            shared,
            results_rx: Mutex::new(results_rx),
            next_seq: AtomicU64::new(1),
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

pub(crate) fn request_status_parse(text: &str) -> u64 {
    let runtime = status_parse_runtime();
    let seq = runtime.next_seq.fetch_add(1, Ordering::Relaxed);
    let mut pending = runtime.shared.pending.lock().unwrap();
    *pending = Some(PendingStatusParse {
        seq,
        text: text.to_string(),
    });
    runtime.shared.wake.notify_one();
    seq
}

pub(crate) fn try_recv_status_parse_result() -> Option<StatusParseResult> {
    let receiver = status_parse_runtime().results_rx.lock().ok()?;
    match receiver.try_recv() {
        Ok(result) => Some(result),
        Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
    }
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

pub(crate) fn write_i3bar_click_event<W: Write>(
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

pub(crate) fn flush_i3bar_click_events<W: Write>(
    writer: &mut W,
    first_event: &mut bool,
) -> std::io::Result<()> {
    while let Some(event) = try_recv_i3bar_click_event() {
        write_i3bar_click_event(&mut *writer, &event, first_event)?;
    }
    writer.flush()
}

pub(crate) fn set_internal_status_ping(ping: calloop::ping::Ping) {
    let runtime = internal_status_runtime();
    if let Ok(mut slot) = runtime.ping.lock() {
        *slot = Some(ping);
    }
}

pub(crate) fn enqueue_internal_status(text: String) {
    let runtime = internal_status_runtime();
    let _ = runtime.sender.send(text);
    if let Ok(guard) = runtime.ping.lock()
        && let Some(ping) = guard.as_ref()
    {
        ping.ping();
    }
}

pub(crate) fn apply_status_update(wm: &mut crate::wm::Wm, text: String) {
    if !text.starts_with("instantwm-") {
        CUSTOM_STATUS_RECEIVED.store(true, Ordering::Relaxed);
    }

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

pub(super) fn send_status_update(text: &str) {
    if INTERNAL_STATUS_RUNTIME.get().is_some() {
        enqueue_internal_status(text.to_string());
    } else {
        send_status_ipc(text);
    }
}

fn send_status_ipc(text: &str) {
    let socket = std::env::var("INSTANTWM_SOCKET")
        .unwrap_or_else(|_| format!("/tmp/instantwm-{}.sock", unsafe { libc::geteuid() }));

    if let Ok(mut stream) = UnixStream::connect(&socket) {
        let req = crate::ipc_types::IpcRequest::new(crate::ipc_types::IpcCommand::UpdateStatus(
            text.to_string(),
        ));
        if let Ok(data) = bincode::encode_to_vec(&req, bincode::config::standard()) {
            let _ = stream.write_all(&data);
        }
    }
}
