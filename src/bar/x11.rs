use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::contexts::CoreCtx;
use crate::types::{Monitor, Systray, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::Window;

pub fn update_status(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
    systray: Option<&mut crate::types::Systray>,
) {
    let selmon_idx = core.globals().selected_monitor_id();

    draw_bar(core, x11_runtime, None, selmon_idx);

    crate::systray::update_systray(core, x11, x11_runtime, systray);
}

pub fn draw_bar(
    core: &mut CoreCtx,
    x11_runtime: &mut X11RuntimeConfig,
    systray: Option<&Systray>,
    mon_idx: usize,
) {
    let bar_win = core
        .globals()
        .monitor(mon_idx)
        .map(|m| m.bar_win)
        .unwrap_or_default();
    if bar_win == WindowId::default() {
        return;
    }
    let work_rect_w = match core.globals().monitor(mon_idx) {
        Some(m) => m.work_rect.w,
        None => return,
    };
    let bar_height = core.globals().cfg.bar_height;
    if work_rect_w <= 0 || bar_height <= 0 {
        return;
    }

    if core.globals().cfg.show_systray {
        core.globals_mut().bar_runtime.systray_width =
            crate::systray::get_systray_width(core, systray) as i32;
    }

    let drw = {
        let Some(drw) = x11_runtime.draw.as_mut() else {
            return;
        };
        if !drw.has_display() {
            return;
        }
        drw.resize(work_rect_w as u32, bar_height as u32);
        drw.clone()
    };

    let mut painter = crate::bar::x11_painter::X11BarPainter::new(drw);

    crate::bar::renderer::draw_bar(core, mon_idx, &mut painter);

    painter.map(bar_win, 0, 0, work_rect_w as u16, bar_height as u16);
}

pub fn draw_bars_x11(
    core: &mut CoreCtx,
    x11_runtime: &mut X11RuntimeConfig,
    systray: Option<&Systray>,
) {
    let indices: Vec<usize> = core.globals().monitors_iter().map(|(i, _)| i).collect();
    for i in indices {
        draw_bar(core, x11_runtime, systray, i);
    }
}

pub fn reset_bar_x11(
    core: &mut CoreCtx,
    x11_runtime: &mut X11RuntimeConfig,
    systray: Option<&Systray>,
) {
    let selmon_idx = core.globals().selected_monitor_id();
    crate::bar::renderer::reset_bar_common(core);
    draw_bar(core, x11_runtime, systray, selmon_idx);
}

/// Resize bar window with dependency injection.
pub fn resize_bar_win(
    core: &CoreCtx,
    x11: &X11BackendRef,
    _x11_runtime: &X11RuntimeConfig,
    systray: Option<&Systray>,
    m: &Monitor,
) {
    // Note: x11_runtime is not mutated here, we only read from it.
    // The systray width calculation only needs immutable access.
    let bar_height = core.globals().cfg.bar_height;
    let showsystray = core.globals().cfg.show_systray;
    let is_selmon = core.globals().selected_monitor().num == m.num;

    let mut w = m.work_rect.w as u32;
    if showsystray && is_selmon {
        w = w.saturating_sub(crate::systray::get_systray_width(core, systray));
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

pub fn update_bars(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
    systray: Option<&Systray>,
) {
    use crate::bar::color::rgba_to_u32;

    let (bar_configs, xlibdisplay, root, status_bg) = {
        let bar_height = core.globals().cfg.bar_height;
        let showsystray = core.globals().cfg.show_systray;
        let status_bg = rgba_to_u32(core.globals().cfg.statusbarcolors.bg);
        let xlibdisplay = x11_runtime.xlibdisplay.0;
        let root = x11_runtime.root;
        let selected_monitor_id = core.globals().selected_monitor_id();

        // Collect systray widths first to avoid borrow issues
        let mut systray_widths: std::collections::HashMap<usize, u32> =
            std::collections::HashMap::new();
        if showsystray {
            systray_widths.insert(
                selected_monitor_id,
                crate::systray::get_systray_width(core, systray),
            );
        }

        let mut bar_configs = Vec::new();
        for (i, m) in core.globals().monitors_iter() {
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
    // borrow conflicts between the X11 connection ref and ctx.globals().
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
        if let Some(mon) = core.globals_mut().monitor_mut(i) {
            mon.bar_win = WindowId::from(win_id);
        }
    }
}
