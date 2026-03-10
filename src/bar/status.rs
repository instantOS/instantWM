use crate::contexts::CoreCtx;
use crate::globals::X11RuntimeConfig;
use crate::types::{Monitor, Rect, Systray};

pub(crate) const MAX_COMMAND_OFFSETS: usize = 20;
pub(crate) const TEXT_PADDING: i32 = 6;
use serde_json::Value;

#[derive(Debug, Clone)]
pub(crate) enum StatusItem {
    Text(String),
    SetBg(String),
    SetFg(String),
    ResetColors,
    Rect(Rect),
    Offset(i32),
    CommandOffset,
}

#[derive(Debug, Clone, Copy)]
struct StatusLayout {
    draw_start_x: i32,
    total_width: i32,
}

pub(crate) fn draw_status_bar(
    ctx: &mut CoreCtx,
    x11_runtime: Option<&X11RuntimeConfig>,
    systray: Option<&Systray>,
    m: &Monitor,
    bar_height: i32,
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> (i32, i32) {
    let stext = ctx.g.status_text.as_str();
    if stext.is_empty() {
        return (0, 0);
    }

    let items = ctx.bar.status_items_for_text(stext).to_vec();
    let layout = measure_layout(ctx, x11_runtime, systray, m, items.as_slice(), painter);

    draw_items(
        painter,
        m,
        bar_height,
        items.as_slice(),
        layout,
        ctx.g,
        &mut ctx.bar.command_offsets,
    );

    (layout.draw_start_x, layout.total_width)
}

fn parse_i3bar_json(bytes: &[u8]) -> Option<Vec<StatusItem>> {
    let mut json_str = std::str::from_utf8(bytes).ok()?;
    if json_str.starts_with(',') {
        json_str = &json_str[1..];
    }
    let array: Vec<Value> = serde_json::from_str(json_str).ok()?;
    let mut items = Vec::new();
    for block in array {
        let obj = block.as_object()?;
        if let Some(color) = obj.get("color").and_then(Value::as_str) {
            if color.starts_with('#') {
                items.push(StatusItem::SetFg(color.to_string()));
            }
        }
        if let Some(bg) = obj.get("background").and_then(Value::as_str) {
            if bg.starts_with('#') {
                items.push(StatusItem::SetBg(bg.to_string()));
            }
        }
        if let Some(full_text) = obj.get("full_text").and_then(Value::as_str) {
            items.push(StatusItem::Text(full_text.to_string()));
        }
        if !items.is_empty() {
            items.push(StatusItem::ResetColors);
            items.push(StatusItem::Text(" ".to_string()));
        }
    }
    Some(items)
}

pub(crate) fn parse_status_items(bytes: &[u8]) -> Vec<StatusItem> {
    if let Some(items) = parse_i3bar_json(bytes) {
        return items;
    }

    let mut items = Vec::new();
    let mut i = 0usize;
    let mut text_start = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'^' {
            i += 1;
            continue;
        }

        if i > text_start {
            let text = std::str::from_utf8(&bytes[text_start..i]).unwrap_or("");
            if !text.is_empty() {
                items.push(StatusItem::Text(text.to_string()));
            }
        }

        i += 1;
        if i >= bytes.len() {
            break;
        }

        if bytes[i] == b'^' {
            items.push(StatusItem::Text("^".to_string()));
            i += 1;
            text_start = i;
            continue;
        }

        let cmd = bytes[i];
        i += 1;

        match cmd {
            b'c' => {
                if i + 7 <= bytes.len() {
                    if let Ok(color) = std::str::from_utf8(&bytes[i..i + 7]) {
                        items.push(StatusItem::SetBg(color.to_string()));
                    }
                    i += 7;
                }
            }
            b't' => {
                if i + 7 <= bytes.len() {
                    if let Ok(color) = std::str::from_utf8(&bytes[i..i + 7]) {
                        items.push(StatusItem::SetFg(color.to_string()));
                    }
                    i += 7;
                }
            }
            b'd' => items.push(StatusItem::ResetColors),
            b'f' => items.push(StatusItem::Offset(parse_number(bytes, &mut i))),
            b'o' => items.push(StatusItem::CommandOffset),
            b'r' => {
                let x = parse_number(bytes, &mut i);
                consume_comma(bytes, &mut i);
                let y = parse_number(bytes, &mut i);
                consume_comma(bytes, &mut i);
                let w = parse_number(bytes, &mut i);
                consume_comma(bytes, &mut i);
                let h = parse_number(bytes, &mut i);
                items.push(StatusItem::Rect(Rect { x, y, w, h }));
            }
            _ => {}
        }

        if i < bytes.len() && bytes[i] == b'^' {
            i += 1;
        }
        text_start = i;
    }

    if text_start < bytes.len() {
        let text = std::str::from_utf8(&bytes[text_start..]).unwrap_or("");
        if !text.is_empty() {
            items.push(StatusItem::Text(text.to_string()));
        }
    }

    items
}

fn consume_comma(bytes: &[u8], i: &mut usize) {
    if *i < bytes.len() && bytes[*i] == b',' {
        *i += 1;
    }
}

fn parse_number(bytes: &[u8], i: &mut usize) -> i32 {
    let start = *i;
    while *i < bytes.len() && (bytes[*i].is_ascii_digit() || bytes[*i] == b'-') {
        *i += 1;
    }
    if *i > start {
        std::str::from_utf8(&bytes[start..*i])
            .ok()
            .and_then(|n| n.parse::<i32>().ok())
            .unwrap_or(0)
    } else {
        0
    }
}

fn measure_layout(
    ctx: &CoreCtx,
    x11_runtime: Option<&X11RuntimeConfig>,
    systray: Option<&Systray>,
    m: &Monitor,
    items: &[StatusItem],
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> StatusLayout {
    let mut width = 0i32;

    for item in items {
        match item {
            StatusItem::Text(text) => width += painter.text_width(text),
            StatusItem::Offset(offset) => width += *offset,
            _ => {}
        }
    }

    let draw_width = (width + 2).max(0);
    let is_selmon = ctx.g.selected_monitor().num == m.num;
    let x11_present = x11_runtime
        .map(|r| !r.xlibdisplay.0.is_null())
        .unwrap_or(false);
    let systray_w = if ctx.g.cfg.show_systray && is_selmon {
        crate::systray::get_systray_width_for_bar(ctx, x11_present, systray)
    } else {
        0
    };
    let draw_start_x = m.work_rect.w - draw_width - systray_w;

    StatusLayout {
        draw_start_x,
        total_width: width.max(0),
    }
}

fn draw_items(
    painter: &mut dyn crate::bar::paint::BarPainter,
    m: &Monitor,
    bar_height: i32,
    items: &[StatusItem],
    layout: StatusLayout,
    g: &crate::globals::Globals,
    command_offsets: &mut [i32; MAX_COMMAND_OFFSETS],
) {
    let mut scheme = g.status_scheme();
    let base_scheme = scheme.clone();

    painter.set_scheme(scheme.clone());

    let draw_width = (layout.total_width + 2).max(0);
    if draw_width > 0 {
        painter.rect(layout.draw_start_x, 0, draw_width, bar_height, true, true);
    }

    command_offsets.fill(-1);

    let mut x = layout.draw_start_x + 1;
    let mut marker_idx = 0usize;

    for item in items {
        match item {
            StatusItem::Text(text) => {
                let seg_w = painter.text_width(text);
                if seg_w > 0 {
                    painter.text(x, 0, seg_w, bar_height, 0, text, false, 0);
                }
                x += seg_w;
            }
            StatusItem::Offset(offset) => x += *offset,
            StatusItem::SetBg(color) => {
                if let Some(clr) = crate::bar::theme::rgba_from_config(color) {
                    scheme.bg = clr;
                    painter.set_scheme(scheme.clone());
                }
            }
            StatusItem::SetFg(color) => {
                if let Some(clr) = crate::bar::theme::rgba_from_config(color) {
                    scheme.fg = clr;
                    painter.set_scheme(scheme.clone());
                }
            }
            StatusItem::ResetColors => {
                scheme = base_scheme.clone();
                painter.set_scheme(scheme.clone());
            }
            StatusItem::Rect(r) => {
                let rw = (r.w).max(0) as u32;
                let rh = (r.h).max(0) as u32;
                if rw > 0 && rh > 0 {
                    painter.rect(x + r.x, r.y, rw as i32, rh as i32, true, false);
                }
            }
            StatusItem::CommandOffset => {
                if marker_idx < MAX_COMMAND_OFFSETS {
                    command_offsets[marker_idx] = x;
                    marker_idx += 1;
                }
            }
        }
    }

    if marker_idx < MAX_COMMAND_OFFSETS {
        command_offsets[marker_idx] = -1;
    }

    let _ = m;
}

fn send_status_ipc(text: &str) {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    let socket = std::env::var("INSTANTWM_SOCKET")
        .unwrap_or_else(|_| format!("/tmp/instantwm-{}.sock", unsafe { libc::geteuid() }));

    if let Ok(mut stream) = UnixStream::connect(&socket) {
        let req = crate::ipc_types::IpcCommand::UpdateStatus(text.to_string());
        if let Ok(data) = bincode::encode_to_vec(&req, bincode::config::standard()) {
            let _ = stream.write_all(&data);
        }
    }
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn default_status_text() -> String {
    use std::time::SystemTime;

    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Convert to local time using libc
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
        use std::thread;
        use std::time::Duration;

        // Wait briefly for the IPC socket to be ready.
        thread::sleep(Duration::from_millis(500));

        loop {
            send_status_ipc(&default_status_text());
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

        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if let Ok(line) = line {
                    let text = line.trim();
                    if text == "[" || text.starts_with("{\"version\"") || text.is_empty() {
                        continue;
                    }
                    send_status_ipc(text);
                }
            }
        }
    });
}
