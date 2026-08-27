use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::contexts::CoreCtx;
use crate::types::{Monitor, MonitorId, Rect, WindowId, XEmbedTray};
use std::collections::HashMap;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::Window;

pub fn update_status(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
    systray: &mut Option<crate::types::XEmbedTray>,
) {
    let selmon_idx = core.model().selected_monitor_id();

    crate::backend::x11::systray::update_systray(core, x11, x11_runtime, systray);
    draw_bar(core, x11_runtime, selmon_idx);
}

pub fn draw_bar(core: &mut CoreCtx, x11_runtime: &mut X11RuntimeConfig, mon_idx: MonitorId) {
    let Some(monitor) = core.model().monitor(mon_idx).cloned() else {
        return;
    };
    let bar_win = monitor.bar_win;
    if bar_win == WindowId::default() {
        return;
    }
    let snapshots = crate::bar::scene::build_monitor_snapshots(
        core,
        true,
        core.bar.runtime.external_tray_width,
    );
    let Some(snapshot) = snapshots
        .iter()
        .find(|snapshot| snapshot.monitor_id == mon_idx)
    else {
        return;
    };
    let work_rect_w = snapshot.rect.w;
    let bar_height = snapshot.rect.h;
    if work_rect_w <= 0 || bar_height <= 0 {
        return;
    }

    let drw = {
        let Some(drw) = x11_runtime.draw.as_mut() else {
            return;
        };
        if !drw.has_display() {
            return;
        }
        drw.resize(work_rect_w as u32, bar_height as u32);
        drw
    };

    let mut painter = crate::backend::x11::bar_painter::X11BarPainter::new(drw);
    crate::bar::renderer::draw_bar_snapshot(core, mon_idx, snapshot, &mut painter);

    painter.map(bar_win, Rect::new(0, 0, work_rect_w, bar_height));
}

pub fn draw_bars(core: &mut CoreCtx, x11_runtime: &mut X11RuntimeConfig) {
    let monitor_ids: Vec<MonitorId> = core.model().monitors_iter().map(|(i, _)| i).collect();
    let snapshots = crate::bar::scene::build_monitor_snapshots(
        core,
        true,
        core.bar.runtime.external_tray_width,
    );
    let snapshot_by_monitor_id: HashMap<MonitorId, &crate::bar::scene::MonitorBarSnapshot> =
        snapshots
            .iter()
            .map(|snapshot| (snapshot.monitor_id, snapshot))
            .collect();

    for i in monitor_ids {
        let Some(bar_win) = core.model().monitor(i).map(|monitor| monitor.bar_win) else {
            continue;
        };
        if bar_win == WindowId::default() {
            continue;
        }

        let Some(snapshot) = snapshot_by_monitor_id.get(&i).copied() else {
            continue;
        };
        let work_rect_w = snapshot.rect.w;
        let bar_height = snapshot.rect.h;
        if work_rect_w <= 0 || bar_height <= 0 {
            continue;
        }

        let drw = {
            let Some(drw) = x11_runtime.draw.as_mut() else {
                continue;
            };
            if !drw.has_display() {
                continue;
            }
            drw.resize(work_rect_w as u32, bar_height as u32);
            drw
        };

        let mut painter = crate::backend::x11::bar_painter::X11BarPainter::new(drw);
        crate::bar::renderer::draw_bar_snapshot(core, i, snapshot, &mut painter);
        painter.map(bar_win, Rect::new(0, 0, work_rect_w, bar_height));
    }
    core.bar.mark_drawn();
}

/// Resize bar window with dependency injection.
pub fn resize_bar_win(
    globals: &crate::core_state::CoreState,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    systray: Option<&XEmbedTray>,
    m: &Monitor,
) {
    // Note: x11_runtime is not mutated here, we only read from it.
    // The systray width calculation only needs immutable access.
    let bar_height = globals.derived.bar_height;
    let showsystray = globals.config.systray.show;
    let is_selmon = globals.model.expect_selected_monitor().num == m.num;

    let mut w = m.work_rect().w as u32;
    if showsystray && is_selmon {
        w = w.saturating_sub(crate::backend::x11::systray::get_systray_width(
            &globals.config.systray,
            globals.derived.bar_height,
            systray,
        ));
    }

    let x11_bar_win: Window = m.bar_win.into();
    let bounds = Rect::new(m.work_rect().x, m.bar_y(), w as i32, bar_height);
    if let Some(draw) = x11_runtime.draw.as_ref() {
        draw.move_resize_window(x11_bar_win, bounds);
    } else {
        let _ = x11.conn.configure_window(
            x11_bar_win,
            &x11rb::protocol::xproto::ConfigureWindowAux::new()
                .x(bounds.x)
                .y(bounds.y)
                .width(bounds.w as u32)
                .height(bounds.h as u32),
        );
    }
}

/// Move/resize the bottom bar window to its current monitor strip and re-apply
/// its background color. `m.bottom_bar_y()` slides the window off-screen when
/// the bar is hidden, so this doubles as the show/hide operation.
pub fn resize_bottom_bar_win(
    globals: &crate::core_state::CoreState,
    x11: &X11BackendRef,
    _x11_runtime: &X11RuntimeConfig,
    m: &Monitor,
) {
    let bottom_win: Window = m.bottom_bar_win.into();
    if bottom_win == 0 {
        return;
    }
    let status_bg: u32 = globals.config.colors.status.bg.into();
    let _ = x11.conn.change_window_attributes(
        bottom_win,
        &x11rb::protocol::xproto::ChangeWindowAttributesAux::new().background_pixel(status_bg),
    );
    let _ = x11.conn.configure_window(
        bottom_win,
        &x11rb::protocol::xproto::ConfigureWindowAux::new()
            .x(m.work_rect().x)
            .y(m.bottom_bar_y())
            .width(m.work_rect().w as u32)
            .height(m.bottom_bar_height as u32),
    );

    // Position the white indicator child window inside the strip.
    let indicator_win: Window = m.bottom_bar_indicator_win.into();
    if indicator_win != 0 {
        let indicator = m.bottom_bar_indicator_rect();
        let _ = x11.conn.configure_window(
            indicator_win,
            &x11rb::protocol::xproto::ConfigureWindowAux::new()
                .x(indicator.x)
                .y(indicator.y)
                .width(indicator.w as u32)
                .height(indicator.h as u32),
        );
    }
}

pub fn update_bars(
    globals: &mut crate::core_state::CoreState,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    systray: Option<&XEmbedTray>,
) {
    let (bar_configs, xlibdisplay, root, status_bg) = {
        let bar_height = globals.derived.bar_height;
        let showsystray = globals.config.systray.show;
        let status_bg: u32 = globals.config.colors.status.bg.into();
        let xlibdisplay = x11_runtime.xlibdisplay.0;
        let root = x11_runtime.root;
        let selected_monitor_id = globals.model.selected_monitor_id();

        // Collect systray widths first to avoid borrow issues
        let mut systray_widths: HashMap<MonitorId, u32> = HashMap::new();
        if showsystray {
            systray_widths.insert(
                selected_monitor_id,
                crate::backend::x11::systray::get_systray_width(
                    &globals.config.systray,
                    globals.derived.bar_height,
                    systray,
                ),
            );
        }

        let mut bar_configs = Vec::new();
        for (i, m) in globals.model.monitors_iter() {
            if m.bar_win != WindowId::default() {
                continue;
            }

            let mut w = m.work_rect().w as u32;
            if showsystray && selected_monitor_id == i {
                w = w.saturating_sub(*systray_widths.get(&i).unwrap_or(&0));
            }
            bar_configs.push((i, m.work_rect().x, m.bar_y(), w, bar_height));
        }
        (bar_configs, xlibdisplay, root, status_bg)
    };

    if xlibdisplay.is_null() {
        return;
    }

    // Create bar windows for each monitor that needs one.
    // We collect window IDs first, then assign them to monitors to avoid
    // borrow conflicts between the X11 connection ref and ctx.state().
    let mut created: Vec<(MonitorId, u32)> = Vec::new();

    let conn = x11.conn;
    for (i, wx, bar_y, w, bar_height) in &bar_configs {
        let win_id = conn
            .generate_id()
            .expect("failed to generate X11 window ID for bar");

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

    // Bottom bar strips: plain override-redirect backgrounds, one per monitor.
    // They select no input events, so button events propagate to the root,
    // where the WM classifies and swallows presses inside the strip.
    let mut bottom_created: Vec<(MonitorId, u32)> = Vec::new();
    for (i, m) in globals.model.monitors_iter() {
        if m.bottom_bar_win != WindowId::default() {
            continue;
        }
        let win_id = conn
            .generate_id()
            .expect("failed to generate X11 window ID for bottom bar");

        let aux = x11rb::protocol::xproto::CreateWindowAux::new()
            .override_redirect(1)
            .background_pixel(status_bg);

        let _ = conn.create_window(
            x11rb::COPY_FROM_PARENT as u8,
            win_id,
            root,
            m.work_rect().x as i16,
            m.bottom_bar_y() as i16,
            m.work_rect().w as u16,
            m.bottom_bar_height as u16,
            0,
            x11rb::protocol::xproto::WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &aux,
        );

        let _ = conn.map_window(win_id);
        let _ = conn.flush();
        bottom_created.push((i, win_id));
    }

    // Create a white indicator child window for each newly-created bottom bar.
    for (i, bottom_id) in &bottom_created {
        let m = globals.model.monitor(*i).unwrap();
        let indicator = m.bottom_bar_indicator_rect();
        let ind_win = conn
            .generate_id()
            .expect("failed to generate X11 window ID for bottom bar indicator");
        let _ = conn.create_window(
            x11rb::COPY_FROM_PARENT as u8,
            ind_win,
            *bottom_id,
            indicator.x as i16,
            indicator.y as i16,
            indicator.w as u16,
            indicator.h as u16,
            0,
            x11rb::protocol::xproto::WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &x11rb::protocol::xproto::CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(0xffffff),
        );
        let _ = conn.map_window(ind_win);
        if let Some(mon) = globals.model.monitor_mut(*i) {
            mon.bottom_bar_indicator_win = WindowId::from(ind_win);
        }
    }

    for (i, win_id) in created {
        if let Some(mon) = globals.model.monitor_mut(i) {
            mon.bar_win = WindowId::from(win_id);
        }
    }
    // Assign bottom windows, then refresh every existing bottom window's
    // geometry/background (reloads, monitor moves, config color changes).
    for (i, win_id) in bottom_created {
        if let Some(mon) = globals.model.monitor_mut(i) {
            mon.bottom_bar_win = WindowId::from(win_id);
        }
    }
    for (_, m) in globals.model.monitors_iter() {
        if m.bottom_bar_win != WindowId::default() {
            resize_bottom_bar_win(globals, x11, x11_runtime, m);
        }
    }
}
