use crate::bar::color::hex_to_u32;
use crate::contexts::WmCtx;
use crate::types::{Monitor, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::Window;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn update_status(ctx: &mut WmCtx) {
    if ctx.x11_conn().is_none() {
        return;
    }
    let root = ctx.g.cfg.root;

    let text = get_text_prop(ctx, root, x11rb::protocol::xproto::AtomEnum::WM_NAME.into());
    match text {
        Some(t) => {
            if t.starts_with("ipc:") {
                return;
            }
            ctx.g.status_text = t;
        }
        None => {
            ctx.g.status_text = format!("instantwm-{}", VERSION);
        }
    }

    let selmon_idx = ctx.g.selected_monitor_id();
    super::draw_bar(ctx, selmon_idx);

    crate::systray::update_systray(ctx);
}

/// Resize bar window with dependency injection.
pub fn resize_bar_win(ctx: &WmCtx, m: &Monitor) {
    let bar_height = ctx.g.cfg.bar_height;
    let showsystray = ctx.g.cfg.showsystray;
    let is_selmon = ctx.g.selected_monitor().num == m.num;

    let mut w = m.work_rect.w as u32;
    if showsystray && is_selmon {
        w = w.saturating_sub(crate::systray::get_systray_width(ctx));
    }

    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return;
    };
    let x11_barwin: Window = m.barwin.into();
    let _ = conn.configure_window(
        x11_barwin,
        &x11rb::protocol::xproto::ConfigureWindowAux::new()
            .x(m.work_rect.x)
            .y(m.by)
            .width(w)
            .height(bar_height as u32),
    );
}

pub fn update_bars(ctx: &mut WmCtx) {
    if ctx.x11_conn().is_none() {
        return;
    }
    let (bar_configs, xlibdisplay, root, status_bg) = {
        let bar_height = ctx.g.cfg.bar_height;
        let showsystray = ctx.g.cfg.showsystray;
        let status_bg = hex_to_u32(ctx.g.cfg.statusbarcolors.get(crate::config::ColIndex::Bg));
        let xlibdisplay = ctx.g.cfg.xlibdisplay.0;
        let root = ctx.g.cfg.root;
        let selected_monitor_id = ctx.g.selected_monitor_id();

        // Collect systray widths first to avoid borrow issues
        let mut systray_widths: std::collections::HashMap<usize, u32> =
            std::collections::HashMap::new();
        if showsystray {
            systray_widths.insert(selected_monitor_id, crate::systray::get_systray_width(ctx));
        }

        let mut bar_configs = Vec::new();
        for (i, m) in ctx.g.monitors_iter() {
            if m.barwin != WindowId::default() {
                continue;
            }

            let mut w = m.work_rect.w as u32;
            if showsystray && selected_monitor_id == i {
                w = w.saturating_sub(*systray_widths.get(&i).unwrap_or(&0));
            }
            bar_configs.push((i, m.work_rect.x, m.by, w, bar_height));
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

    if let Some(x11) = ctx.x11_conn() {
        let conn = x11.conn;
        for (i, wx, by, w, bar_height) in &bar_configs {
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
                *by as i16,
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
    }

    for (i, win_id) in created {
        if let Some(mon) = ctx.g.monitor_mut(i) {
            mon.barwin = WindowId::from(win_id);
        }
    }
}

pub fn toggle_bar(ctx: &mut WmCtx) {
    let animated = ctx.g.animated;
    let client_count = ctx.g.clients.len() as i32;
    let mut tmp_no_anim = false;
    if animated && client_count > 6 {
        ctx.g.animated = false;
        tmp_no_anim = true;
    }

    let bar_height = ctx.g.cfg.bar_height;
    let selmon = ctx.g.selected_monitor_mut();
    selmon.showbar = !selmon.showbar;

    let current_tag = selmon.current_tag;
    if current_tag > 0 && current_tag <= selmon.tags.len() {
        selmon.tags[current_tag - 1].showbar = selmon.showbar;
    }

    selmon.update_bar_position(bar_height);

    let selmon_idx = ctx.g.selected_monitor_id();
    if let Some(m) = ctx.g.monitor(selmon_idx) {
        resize_bar_win(ctx, m);
    }

    if tmp_no_anim {
        ctx.g.animated = true;
    }
}

fn get_text_prop(ctx: &WmCtx, win: Window, atom: u32) -> Option<String> {
    let conn = ctx.x11_conn().map(|x11| x11.conn)?;
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
