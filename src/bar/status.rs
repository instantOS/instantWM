#![allow(clippy::large_enum_variant)]
use crate::contexts::CoreCtx;
use crate::types::Monitor;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

pub(crate) const TEXT_PADDING: i32 = 6;
const DEFAULT_SEPARATOR_BLOCK_WIDTH: i32 = 9;

pub static CUSTOM_STATUS_RECEIVED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[derive(Debug, Clone)]
pub(crate) enum StatusItem {
    Text(String),
    I3Block(I3Block),
}

#[derive(Debug, Clone)]
pub(crate) struct I3Block {
    pub full_text: String,
    pub short_text: Option<String>,
    pub color: Option<String>,
    pub background: Option<String>,
    pub border: Option<String>,
    pub border_top: i32,
    pub border_right: i32,
    pub border_bottom: i32,
    pub border_left: i32,
    pub min_width: Option<I3MinWidth>,
    pub align: I3Align,
    pub urgent: bool,
    pub separator: bool,
    pub separator_block_width: i32,
    pub name: Option<String>,
    pub instance: Option<String>,
    pub markup: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum I3MinWidth {
    Text(String),
    Pixels(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum I3Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct I3StatusLine {
    pub blocks: Vec<I3Block>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct I3BarHeader {
    pub version: Option<i32>,
    pub click_events: bool,
    pub stop_signal: Option<i32>,
    pub cont_signal: Option<i32>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StatusClickTarget {
    pub start_x: i32,
    pub end_x: i32,
    pub index: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ParsedStatus {
    pub items: Vec<StatusItem>,
    pub i3bar: Option<I3StatusLine>,
}

#[derive(Debug, Clone)]
struct PendingStatusParse {
    seq: u64,
    text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct StatusParseResult {
    pub seq: u64,
    pub text: String,
    pub parsed: ParsedStatus,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct I3ClickEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    pub button: u8,
    pub x: i32,
    pub y: i32,
    pub relative_x: i32,
    pub relative_y: i32,
    pub width: i32,
    pub height: i32,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub modifiers: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct StatusLayout {
    draw_start_x: i32,
    total_width: i32,
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

static I3BAR_CLICK_RUNTIME: OnceLock<I3ClickRuntime> = OnceLock::new();
static STATUS_PARSE_RUNTIME: OnceLock<StatusParseRuntime> = OnceLock::new();

#[derive(Debug)]
struct InternalStatusRuntime {
    sender: Sender<String>,
    receiver: Mutex<Receiver<String>>,
    ping: Mutex<Option<calloop::ping::Ping>>,
}

static INTERNAL_STATUS_RUNTIME: OnceLock<InternalStatusRuntime> = OnceLock::new();

#[derive(Debug, Clone, Deserialize)]
struct RawI3Block {
    #[serde(default)]
    full_text: String,
    #[serde(default)]
    short_text: Option<String>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    background: Option<String>,
    #[serde(default)]
    border: Option<String>,
    #[serde(default)]
    border_top: Option<i32>,
    #[serde(default)]
    border_right: Option<i32>,
    #[serde(default)]
    border_bottom: Option<i32>,
    #[serde(default)]
    border_left: Option<i32>,
    #[serde(default)]
    min_width: Option<Value>,
    #[serde(default)]
    align: Option<String>,
    #[serde(default)]
    urgent: bool,
    #[serde(default = "default_true")]
    separator: bool,
    #[serde(default)]
    separator_block_width: Option<i32>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    instance: Option<String>,
    #[serde(default)]
    markup: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Draw the status bar.
///
/// The `systray_width` parameter is pre-calculated by the caller to avoid
/// backend-specific dependencies in this function.
pub(crate) fn draw_status_bar(
    ctx: &mut CoreCtx,
    systray_width: i32,
    m: &Monitor,
    bar_height: i32,
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> (i32, i32, Vec<StatusClickTarget>) {
    let mode = ctx.globals().behavior.current_mode.clone();
    let stext_owned: String;
    let stext = if !mode.is_empty() && mode != "default" {
        // Try to get mode description, fall back to mode name
        let mode_display = ctx
            .globals()
            .cfg
            .modes
            .get(&mode)
            .and_then(|m| m.description.as_ref())
            .map(|s| s.as_str())
            .unwrap_or(&mode);
        stext_owned = format!("mode: {}", mode_display);
        stext_owned.as_str()
    } else {
        stext_owned = ctx.globals().bar_runtime.status_text.clone();
        stext_owned.as_str()
    };

    if stext.is_empty() {
        return (0, 0, Vec::new());
    }

    let items = ctx.bar.status_items_for_text(stext).to_vec();
    draw_status_items(
        systray_width,
        m,
        bar_height,
        items.as_slice(),
        ctx.globals().status_scheme(),
        painter,
    )
}

fn parse_i3bar_json(bytes: &[u8]) -> Option<ParsedStatus> {
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
    // Try i3bar JSON format first
    if let Some(parsed) = parse_i3bar_json(bytes) {
        return parsed;
    }

    // Fall back to plain text
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

pub(crate) fn hit_test_i3_click_target(
    click_targets: &[StatusClickTarget],
    local_x: i32,
) -> Option<usize> {
    click_targets
        .iter()
        .find(|target| local_x >= target.start_x && local_x < target.end_x)
        .map(|target| target.index)
}

pub(crate) fn resolve_i3_click<'a>(
    parsed: &'a ParsedStatus,
    click_targets: &[StatusClickTarget],
    local_x: i32,
) -> Option<(&'a I3Block, StatusClickTarget)> {
    let line = parsed.i3bar.as_ref()?;
    let block_idx = hit_test_i3_click_target(click_targets, local_x)?;
    let block = line.blocks.get(block_idx)?;
    let target = click_targets
        .iter()
        .copied()
        .find(|target| target.index == block_idx)?;

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
    if mask & crate::config::keybindings::MODKEY != 0 {
        modifiers.push("Mod4".to_string());
    }

    modifiers
}

pub(crate) fn make_i3_click_event(
    block: &I3Block,
    target: StatusClickTarget,
    button: u8,
    x: i32,
    y: i32,
    bar_height: i32,
    clean_state: u32,
) -> I3ClickEvent {
    I3ClickEvent {
        name: block.name.clone(),
        instance: block.instance.clone(),
        button,
        x,
        y,
        relative_x: x - target.start_x,
        relative_y: y.max(0),
        width: (target.end_x - target.start_x).max(0),
        height: bar_height.max(0),
        modifiers: modifiers_from_mask(clean_state),
    }
}

pub(crate) fn emit_i3bar_status_click(
    parsed: &ParsedStatus,
    click_targets: &[StatusClickTarget],
    local_x: i32,
    y: i32,
    button: u8,
    bar_height: i32,
    clean_state: u32,
) -> bool {
    let Some((block, target)) = resolve_i3_click(parsed, click_targets, local_x) else {
        return false;
    };

    enqueue_i3bar_click_event(make_i3_click_event(
        block,
        target,
        button,
        local_x,
        y,
        bar_height,
        clean_state,
    ));
    true
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
pub(crate) fn enqueue_i3bar_click_event(event: I3ClickEvent) {
    let _ = i3bar_click_runtime().sender.send(event);
}

pub(crate) fn try_recv_i3bar_click_event() -> Option<I3ClickEvent> {
    let receiver = i3bar_click_runtime().receiver.lock().ok()?;
    match receiver.try_recv() {
        Ok(event) => Some(event),
        Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
    }
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
        CUSTOM_STATUS_RECEIVED.store(true, std::sync::atomic::Ordering::Relaxed);
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

fn measure_layout(
    systray_width: i32,
    m: &Monitor,
    items: &[StatusItem],
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> StatusLayout {
    let mut width = 0i32;

    for item in items {
        match item {
            StatusItem::Text(text) => width += painter.text_width(text),
            StatusItem::I3Block(block) => width += measure_i3_block_width(block, painter),
        }
    }

    let draw_width = (width + 2).max(0);
    let draw_start_x = m.work_rect.w - draw_width - systray_width;

    StatusLayout {
        draw_start_x,
        total_width: width.max(0),
    }
}

fn measure_i3_block_width(block: &I3Block, painter: &mut dyn crate::bar::paint::BarPainter) -> i32 {
    let text = block_render_text(block);
    let text_width = painter.text_width(text);

    let min_width = match &block.min_width {
        Some(I3MinWidth::Text(s)) => painter.text_width(s),
        Some(I3MinWidth::Pixels(px)) => *px,
        None => 0,
    };

    let padding = if !block.separator && block.separator_block_width == 0 {
        0
    } else {
        TEXT_PADDING
    };

    let content_width = text_width.max(min_width).max(0);
    let border_width = block.border_left + block.border_right;
    let block_width = border_width + padding * 2 + content_width;

    let separator_width = if block.separator {
        block.separator_block_width
    } else {
        0
    };

    (block_width + separator_width).max(0)
}

pub(crate) fn draw_status_items(
    systray_width: i32,
    m: &Monitor,
    bar_height: i32,
    items: &[StatusItem],
    base_scheme: crate::bar::paint::BarScheme,
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> (i32, i32, Vec<StatusClickTarget>) {
    let layout = measure_layout(systray_width, m, items, painter);
    let mut click_targets = Vec::new();
    draw_items(
        painter,
        bar_height,
        items,
        layout,
        &base_scheme,
        &mut click_targets,
    );
    (layout.draw_start_x, layout.total_width, click_targets)
}

fn draw_items(
    painter: &mut dyn crate::bar::paint::BarPainter,
    bar_height: i32,
    items: &[StatusItem],
    layout: StatusLayout,
    base_scheme: &crate::bar::paint::BarScheme,
    click_targets: &mut Vec<StatusClickTarget>,
) {
    painter.set_scheme(base_scheme.clone());

    let draw_width = (layout.total_width + 2).max(0);
    if draw_width > 0 {
        painter.rect(layout.draw_start_x, 0, draw_width, bar_height, true, true);
    }

    click_targets.clear();

    let mut x = layout.draw_start_x + 1;
    let mut click_idx = 0usize;

    for item in items {
        match item {
            StatusItem::Text(text) => {
                let seg_w = painter.text_width(text);
                if seg_w > 0 {
                    painter.text(x, 0, seg_w, bar_height, 0, text, false, 0);
                }
                x += seg_w;
            }
            StatusItem::I3Block(block) => {
                let total_w = draw_i3_block(painter, x, bar_height, block, base_scheme);
                if total_w > 0 {
                    click_targets.push(StatusClickTarget {
                        start_x: x,
                        end_x: x + total_w,
                        index: click_idx,
                    });
                }
                x += total_w;
                click_idx += 1;
            }
        }
    }
}

fn draw_i3_block(
    painter: &mut dyn crate::bar::paint::BarPainter,
    x: i32,
    bar_height: i32,
    block: &I3Block,
    base_scheme: &crate::bar::paint::BarScheme,
) -> i32 {
    let mut fg = block
        .color
        .as_deref()
        .and_then(crate::bar::theme::rgba_from_config)
        .unwrap_or(base_scheme.fg);
    let mut bg = block
        .background
        .as_deref()
        .and_then(crate::bar::theme::rgba_from_config)
        .unwrap_or(base_scheme.bg);
    let mut detail = base_scheme.detail;

    if block.urgent {
        std::mem::swap(&mut fg, &mut bg);
        detail = fg;
    }

    let block_scheme = crate::bar::paint::BarScheme { fg, bg, detail };
    painter.set_scheme(block_scheme.clone());

    let text = block_render_text(block);
    let text_width = painter.text_width(text);
    let min_width = match &block.min_width {
        Some(I3MinWidth::Text(s)) => painter.text_width(s),
        Some(I3MinWidth::Pixels(px)) => *px,
        None => 0,
    };

    let padding = if !block.separator && block.separator_block_width == 0 {
        0
    } else {
        TEXT_PADDING
    };

    let content_width = text_width.max(min_width).max(0);
    let block_inner_width = (padding * 2 + content_width).max(0);
    let block_width = (block.border_left + block.border_right + block_inner_width).max(0);
    if block_width <= 0 {
        return 0;
    }

    painter.rect(x, 0, block_width, bar_height, true, true);

    let border_color = block
        .border
        .as_deref()
        .and_then(crate::bar::theme::rgba_from_config)
        .unwrap_or(block_scheme.detail);

    let border_scheme = crate::bar::paint::BarScheme {
        fg: border_color,
        bg: block_scheme.bg,
        detail: border_color,
    };
    painter.set_scheme(border_scheme);

    if block.border_top > 0 {
        painter.rect(
            x,
            0,
            block_width,
            block.border_top.min(bar_height),
            true,
            false,
        );
    }
    if block.border_bottom > 0 {
        let h = block.border_bottom.min(bar_height);
        painter.rect(x, bar_height - h, block_width, h, true, false);
    }
    if block.border_left > 0 {
        painter.rect(
            x,
            0,
            block.border_left.min(block_width),
            bar_height,
            true,
            false,
        );
    }
    if block.border_right > 0 {
        let w = block.border_right.min(block_width);
        painter.rect(x + block_width - w, 0, w, bar_height, true, false);
    }

    painter.set_scheme(block_scheme);

    let text_area_x = x + block.border_left + padding;
    let text_area_width =
        (block_width - block.border_left - block.border_right - padding * 2).max(0);

    if text_area_width > 0 {
        let lpad = match block.align {
            I3Align::Left => 0,
            I3Align::Center => ((text_area_width - text_width) / 2).max(0),
            I3Align::Right => (text_area_width - text_width).max(0),
        };
        painter.text(
            text_area_x,
            0,
            text_area_width,
            bar_height,
            lpad,
            text,
            false,
            0,
        );
    }

    let separator_width = if block.separator {
        block.separator_block_width
    } else {
        0
    };

    if separator_width > 0 {
        painter.set_scheme(base_scheme.clone());
        painter.rect(x + block_width, 0, separator_width, bar_height, true, true);

        let line_h = (bar_height - 8).max(4);
        let line_y = (bar_height - line_h) / 2;
        let line_x = x + block_width + separator_width / 2;
        painter.rect(line_x, line_y, 1, line_h, true, false);
    }

    block_width + separator_width
}

fn block_render_text(block: &I3Block) -> &str {
    block
        .short_text
        .as_deref()
        .unwrap_or(block.full_text.as_str())
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

fn send_status_update(text: &str) {
    let has_internal_runtime = INTERNAL_STATUS_RUNTIME.get().is_some();
    if has_internal_runtime {
        enqueue_internal_status(text.to_string());
    } else {
        send_status_ipc(text);
    }
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn default_status_text() -> String {
    use std::time::SystemTime;

    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let time_str = unsafe {
        let secs_i64 = secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs_i64, &mut tm);
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
    };

    format!("instantwm-{VERSION} {time_str}")
}

/// Spawn a background thread that periodically sends the default status
/// (version + current time) via IPC. Used when no `status_command` is configured.
pub(crate) fn spawn_default_status() {
    std::thread::spawn(move || {
        use std::sync::atomic::Ordering;
        use std::thread;
        use std::time::Duration;

        thread::sleep(Duration::from_millis(500));

        loop {
            if CUSTOM_STATUS_RECEIVED.load(Ordering::Relaxed) {
                break;
            }
            send_status_update(&default_status_text());
            thread::sleep(Duration::from_secs(30));
        }
    });
}

pub(crate) fn spawn_status_command(cmd: &str) {
    let cmd_str = cmd.to_string();
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};

        let mut child = match Command::new("sh")
            .arg("-c")
            .arg(&cmd_str)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "instantwm: failed to spawn status_command '{}': {}",
                    cmd_str, e
                );
                return;
            }
        };

        let mut i3bar_mode = false;

        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(line) = line else {
                    continue;
                };

                let text = line.trim();
                if text.is_empty() || text == "[" {
                    continue;
                }

                if !i3bar_mode && let Some(header) = parse_i3bar_header(text) {
                    i3bar_mode = true;
                    if header.click_events
                        && let Some(mut stdin) = child.stdin.take()
                    {
                        std::thread::spawn(move || {
                            let mut first_click_event = true;
                            while flush_i3bar_click_events(&mut stdin, &mut first_click_event)
                                .is_ok()
                            {
                                std::thread::sleep(std::time::Duration::from_millis(25));
                            }
                        });
                    }
                    continue;
                }

                if i3bar_mode {
                    if parse_i3bar_json(text.as_bytes()).is_some() {
                        send_status_update(text);
                    } else {
                        log::debug!("dropping malformed i3bar status frame: {text}");
                    }
                } else {
                    send_status_update(text);
                }
            }
        }
    });
}

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
