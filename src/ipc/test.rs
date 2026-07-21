//! Deliberately unstable IPC helpers for profiling and automated tests.

use crate::backend::PointerOps;
use crate::ipc_types::{Response, TestCommand};
use crate::layouts::arrange;
use crate::types::{BaseClientMode, TagMask, WindowId};
use crate::wm::Wm;

pub fn handle_test_command(wm: &mut Wm, command: TestCommand) -> Response {
    if std::env::var("INSTANTWM_TEST").as_deref() != Ok("1") {
        return Response::err("test commands are disabled; start instantWM with INSTANTWM_TEST=1");
    }

    match command {
        TestCommand::PointerMove { x, y, normalized } => move_pointer(wm, x, y, normalized),
        TestCommand::FocusWindow(raw) => focus_window(wm, WindowId(raw)),
        TestCommand::TagWindow { window_id, tag } => tag_window(wm, WindowId(window_id), tag),
        TestCommand::SetWindowFloating {
            window_id,
            floating,
        } => set_window_floating(wm, WindowId(window_id), floating),
    }
}

fn move_pointer(wm: &mut Wm, mut x: f64, mut y: f64, normalized: bool) -> Response {
    if !x.is_finite() || !y.is_finite() {
        return Response::err("pointer coordinates must be finite numbers");
    }

    if normalized {
        if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
            return Response::err("normalized pointer coordinates must be between 0 and 1");
        }
        let rect = wm.core.model.expect_selected_monitor().monitor_rect;
        x = f64::from(rect.x) + x * f64::from((rect.w - 1).max(0));
        y = f64::from(rect.y) + y * f64::from((rect.h - 1).max(0));
    }

    wm.backend.warp_pointer(x, y);
    Response::ok()
}

fn focus_window(wm: &mut Wm, win: WindowId) -> Response {
    if wm.core.model.client(win).is_none() {
        return Response::err(format!("window {} not found", win.0));
    }
    crate::focus::focus(&mut wm.ctx(), Some(win));
    Response::ok()
}

fn tag_window(wm: &mut Wm, win: WindowId, tag: u32) -> Response {
    if wm.core.model.client(win).is_none() {
        return Response::err(format!("window {} not found", win.0));
    }
    let Some(mask) = usize::try_from(tag).ok().and_then(TagMask::single) else {
        return Response::err(format!("invalid tag {tag}"));
    };
    if !mask.intersects(wm.core.model.tags.mask()) {
        return Response::err(format!("tag {tag} is not configured"));
    }
    crate::tags::client_tags::set_client_tag(&mut wm.ctx(), win, mask);
    Response::ok()
}

fn set_window_floating(wm: &mut Wm, win: WindowId, floating: bool) -> Response {
    let Some(monitor_id) = wm.core.model.client(win).map(|client| client.monitor_id) else {
        return Response::err(format!("window {} not found", win.0));
    };
    let mode = if floating {
        BaseClientMode::Floating
    } else {
        BaseClientMode::Tiling
    };
    let _ = crate::floating::set_window_mode(&mut wm.ctx(), win, mode);
    arrange(&mut wm.ctx(), Some(monitor_id));
    Response::ok()
}
