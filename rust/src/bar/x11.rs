use crate::contexts::WmCtx;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::types::Monitor;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::Window;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn update_status(ctx: &mut WmCtx) {
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

    let selmon_idx = ctx.g.selmon;
    if let Some(m) = ctx.g.monitors.get_mut(selmon_idx) {
        super::draw_bar(m);
    }

    crate::systray::update_systray(ctx);
}

pub fn update_bar_pos(m: &mut Monitor) {
    let bh = get_globals().cfg.bh;
    update_bar_pos_with_bh(m, bh);
}

pub(crate) fn update_bar_pos_with_bh(m: &mut Monitor, bh: i32) {
    m.work_rect.y = m.monitor_rect.y;
    m.work_rect.h = m.monitor_rect.h;

    if m.showbar {
        m.work_rect.h -= bh;
        if m.topbar {
            m.by = m.work_rect.y;
            m.work_rect.y += bh;
        } else {
            m.by = m.work_rect.y + m.work_rect.h;
        }
    } else {
        m.by = -bh;
    }
}

pub fn resize_bar_win(m: &Monitor) {
    let g = get_globals();
    let bh = g.cfg.bh;
    let showsystray = g.cfg.showsystray;
    let is_selmon = g
        .monitors
        .get(g.selmon)
        .is_some_and(|selmon| selmon.num == m.num);

    let mut w = m.work_rect.w as u32;
    if showsystray && is_selmon {
        // Use global-based systray width calculation
        w = w.saturating_sub(get_systray_width_static());
    }

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.configure_window(
            m.barwin,
            &x11rb::protocol::xproto::ConfigureWindowAux::new()
                .x(m.work_rect.x)
                .y(m.by)
                .width(w)
                .height(bh as u32),
        );
    }
}

/// Get systray width using global state (for use when ctx is not available)
fn get_systray_width_static() -> u32 {
    let g = get_globals();
    if !g.cfg.showsystray {
        return 1;
    }

    let mut w: u32 = 0;
    if let Some(ref systray) = g.systray {
        for &icon_win in &systray.icons {
            if let Some(c) = g.clients.get(&icon_win) {
                w += c.geo.w as u32 + g.cfg.systrayspacing as u32;
            }
        }
    }

    if w > 0 {
        w + g.cfg.systrayspacing as u32
    } else {
        1
    }
}

/// Resize bar window with dependency injection.
pub fn resize_bar_win_ctx(ctx: &WmCtx, m: &Monitor) {
    let bh = ctx.g.cfg.bh;
    let showsystray = ctx.g.cfg.showsystray;
    let is_selmon = ctx
        .g
        .monitors
        .get(ctx.g.selmon)
        .is_some_and(|selmon| selmon.num == m.num);

    let mut w = m.work_rect.w as u32;
    if showsystray && is_selmon {
        w = w.saturating_sub(crate::systray::get_systray_width(ctx));
    }

    if let Some(ref conn) = ctx.x11.conn {
        let _ = conn.configure_window(
            m.barwin,
            &x11rb::protocol::xproto::ConfigureWindowAux::new()
                .x(m.work_rect.x)
                .y(m.by)
                .width(w)
                .height(bh as u32),
        );
    }
}

pub fn update_bars(ctx: &mut WmCtx) {
    let (bar_configs, xlibdisplay, root, status_bg) = {
        let bh = ctx.g.cfg.bh;
        let showsystray = ctx.g.cfg.showsystray;
        let status_bg = parse_color_to_u32(
            ctx.g
                .cfg
                .statusbarcolors
                .get(1)
                .copied()
                .unwrap_or("#121212"),
        );
        let xlibdisplay = ctx.g.cfg.xlibdisplay.0;
        let root = ctx.g.cfg.root;
        let selmon = ctx.g.selmon;

        // Collect systray widths first to avoid borrow issues
        let mut systray_widths: std::collections::HashMap<usize, u32> =
            std::collections::HashMap::new();
        if showsystray {
            for i in 0..ctx.g.monitors.len() {
                if selmon == i {
                    systray_widths.insert(i, crate::systray::get_systray_width(ctx));
                }
            }
        }

        let mut bar_configs = Vec::new();
        for (i, m) in ctx.g.monitors.iter().enumerate() {
            if m.barwin != 0 {
                continue;
            }

            let mut w = m.work_rect.w as u32;
            if showsystray && selmon == i {
                w = w.saturating_sub(*systray_widths.get(&i).unwrap_or(&0));
            }
            bar_configs.push((i, m.work_rect.x, m.by, w, bh));
        }
        (bar_configs, xlibdisplay, root, status_bg)
    };

    if xlibdisplay.is_null() {
        return;
    }

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        for (i, wx, by, w, bh) in bar_configs {
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
                wx as i16,
                by as i16,
                w as u16,
                bh as u16,
                0,
                x11rb::protocol::xproto::WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &aux,
            );

            let _ = conn.map_window(win_id);
            let _ = conn.flush();

            if let Some(mon) = ctx.g.monitors.get_mut(i) {
                mon.barwin = win_id;
            }
        }
    }
}

pub fn toggle_bar() {
    let g = get_globals_mut();

    let animated = g.animated;
    let client_count = g.clients.len() as i32;
    let mut tmp_no_anim = false;
    if animated && client_count > 6 {
        g.animated = false;
        tmp_no_anim = true;
    }

    let selmon_idx = g.selmon;
    if let Some(selmon) = g.monitors.get_mut(selmon_idx) {
        selmon.showbar = !selmon.showbar;

        let current_tag = selmon.current_tag;
        if current_tag > 0 && current_tag <= g.tags.tags.len() {
            g.tags.tags[current_tag - 1].showbar = selmon.showbar;
        }

        update_bar_pos(selmon);
        let m = selmon.clone();
        resize_bar_win(&m);
    }

    if tmp_no_anim {
        g.animated = true;
    }
}

fn get_text_prop(ctx: &WmCtx, win: Window, atom: u32) -> Option<String> {
    let conn = ctx.x11.conn.as_ref()?;
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

fn parse_color_to_u32(color: &str) -> u32 {
    let color = color.trim_start_matches('#');
    if color.len() == 6 {
        let r = u32::from_str_radix(&color[0..2], 16).unwrap_or(0);
        let g = u32::from_str_radix(&color[2..4], 16).unwrap_or(0);
        let b = u32::from_str_radix(&color[4..6], 16).unwrap_or(0);
        (r << 16) | (g << 8) | b
    } else {
        0x121212
    }
}
