use crate::bar::color::hex_to_u32;
use crate::contexts::{CoreCtx, X11Ctx};
use crate::types::{Monitor, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::Window;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn update_status(core: &mut CoreCtx, x11: &X11Ctx) {
    let root = core.g.x11.root;

    let text = get_text_prop(x11, root, x11rb::protocol::xproto::AtomEnum::WM_NAME.into());
    match text {
        Some(t) => {
            if t.starts_with("ipc:") {
                return;
            }
            core.g.status_text = t;
        }
        None => {
            core.g.status_text = format!("instantwm-{}", VERSION);
        }
    }

    let selmon_idx = core.g.selected_monitor_id();
    super::draw_bar(core, selmon_idx);

    crate::systray::update_systray(core, x11);
}

/// Resize bar window with dependency injection.
pub fn resize_bar_win(core: &CoreCtx, x11: &X11Ctx, m: &Monitor) {
    let bar_height = core.g.cfg.bar_height;
    let showsystray = core.g.cfg.showsystray;
    let is_selmon = core.g.selected_monitor().num == m.num;

    let mut w = m.work_rect.w as u32;
    if showsystray && is_selmon {
        w = w.saturating_sub(crate::systray::get_systray_width(core));
    }

    let conn = x11.conn;
    let x11_bar_win: Window = m.bar_win.into();
    let _ = conn.configure_window(
        x11_bar_win,
        &x11rb::protocol::xproto::ConfigureWindowAux::new()
            .x(m.work_rect.x)
            .y(m.bar_y)
            .width(w)
            .height(bar_height as u32),
    );
}

pub fn update_bars(core: &mut CoreCtx, x11: &X11Ctx) {
    let (bar_configs, xlibdisplay, root, status_bg) = {
        let bar_height = core.g.cfg.bar_height;
        let showsystray = core.g.cfg.showsystray;
        let status_bg = hex_to_u32(core.g.cfg.statusbarcolors.get(crate::config::ColIndex::Bg));
        let xlibdisplay = core.g.x11.xlibdisplay.0;
        let root = core.g.x11.root;
        let selected_monitor_id = core.g.selected_monitor_id();

        // Collect systray widths first to avoid borrow issues
        let mut systray_widths: std::collections::HashMap<usize, u32> =
            std::collections::HashMap::new();
        if showsystray {
            systray_widths.insert(selected_monitor_id, crate::systray::get_systray_width(core));
        }

        let mut bar_configs = Vec::new();
        for (i, m) in core.g.monitors_iter() {
            if m.bar_win != WindowId::default() {
                continue;
            }

            let mut w = m.work_rect.w as u32;
            if showsystray && selected_monitor_id == i {
                w = w.saturating_sub(*systray_widths.get(&i).unwrap_or(&0));
            }
            bar_configs.push((i, m.work_rect.x, m.bar_y, w, bar_height));
        }
        (bar_configs, xlibdisplay, root, status_bg)
    };

    if xlibdisplay.is_null() {
        return;
    }

    // Create bar windows for each monitor that needs one.
    // We collect window IDs first, then assign them to monitors to avoid
    // borrow conflicts between the X11 connection ref and ctx.g.
    let mut created: Vec<(usize, u32)> = Vec::new();

    let conn = x11.conn;
    for (i, wx, bar_y, w, bar_height) in &bar_configs {
        let win_id = conn.generate_id().unwrap();

        let aux = x11rb::protocol::xproto::CreateWindowAux::new()
            .override_redirect(1)
            .background_pixel(status_bg)
            .event_mask(
                x11rb::protocol::xproto::EventMask::BUTTON_PRESS
                    | x11rb::protocol::xproto::EventMask::EXPOSURE
                    | x11rb::protocol::xproto::EventMask::LEAVE_WINDOW,
            );

        let _ = conn.create_window(
            x11rb::COPY_FROM_PARENT as u8,
            win_id,
            root,
            *wx as i16,
            *bar_y as i16,
            *w as u16,
            *bar_height as u16,
            0,
            x11rb::protocol::xproto::WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &aux,
        );

        let _ = conn.map_window(win_id);
        let _ = conn.flush();
        created.push((*i, win_id));
    }

    for (i, win_id) in created {
        if let Some(mon) = core.g.monitor_mut(i) {
            mon.bar_win = WindowId::from(win_id);
        }
    }
}

pub fn toggle_bar(core: &mut CoreCtx, x11: &X11Ctx) {
    let animated = core.g.animated;
    let client_count = core.g.clients.len() as i32;
    let mut tmp_no_anim = false;
    if animated && client_count > 6 {
        core.g.animated = false;
        tmp_no_anim = true;
    }

    let bar_height = core.g.cfg.bar_height;
    let selmon = core.g.selected_monitor_mut();
    selmon.showbar = !selmon.showbar;

    let current_tag = selmon.current_tag;
    if current_tag > 0 && current_tag <= selmon.tags.len() {
        selmon.tags[current_tag - 1].showbar = selmon.showbar;
    }

    selmon.update_bar_position(bar_height);

    let selmon_idx = core.g.selected_monitor_id();
    if let Some(m) = core.g.monitor(selmon_idx) {
        resize_bar_win(core, x11, m);
    }

    if tmp_no_anim {
        core.g.animated = true;
    }
}

fn get_text_prop(x11: &X11Ctx, win: Window, atom: u32) -> Option<String> {
    let conn = x11.conn;
    let reply = conn
        .get_property(
            false,
            win,
            atom,
            x11rb::protocol::xproto::AtomEnum::ANY,
            0,
            4096,
        )
        .ok()?
        .reply()
        .ok()?;
    if reply.format != 8 || reply.value.is_empty() {
        return None;
    }
    let nul_pos = reply
        .value
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(reply.value.len());
    String::from_utf8(reply.value[..nul_pos].to_vec()).ok()
}
