use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::contexts::CoreCtx;
use crate::types::{Monitor, MonitorId, Rect, WindowId, XEmbedTray};
use std::collections::HashMap;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::Window;

#[derive(Clone, Copy)]
struct BarSurfaceTarget {
    monitor_id: MonitorId,
    window_id: WindowId,
}

pub fn update_status(core: &mut CoreCtx, x11_runtime: &mut X11RuntimeConfig) {
    let Some(target) = core
        .model()
        .selected_monitor()
        .map(|monitor| BarSurfaceTarget {
            monitor_id: monitor.id(),
            window_id: monitor.bar_win,
        })
    else {
        return;
    };
    draw_bar(core, x11_runtime, target);
}

fn paint_bar_snapshot(
    core: &mut CoreCtx,
    x11_runtime: &mut X11RuntimeConfig,
    target: BarSurfaceTarget,
    snapshot: &crate::bar::scene::MonitorBarSnapshot,
) {
    let work_rect_w = snapshot.rect.w;
    let bar_height = snapshot.rect.h;
    if work_rect_w <= 0 || bar_height <= 0 {
        return;
    }
    let drw = match x11_runtime.draw.as_mut() {
        Some(drw) if drw.has_display() => drw,
        _ => return,
    };
    drw.resize(work_rect_w as u32, bar_height as u32);
    let mut painter = crate::backend::x11::bar_painter::X11BarPainter::new(drw);
    let hit = crate::bar::scene::render_monitor_snapshot(snapshot, &mut painter);
    core.bar.replace_hit_cache(target.monitor_id, hit);
    painter.map(target.window_id, Rect::new(0, 0, work_rect_w, bar_height));
}

fn draw_bar(core: &mut CoreCtx, x11_runtime: &mut X11RuntimeConfig, target: BarSurfaceTarget) {
    if target.window_id == WindowId::default() {
        return;
    }
    let snapshots =
        crate::bar::scene::build_monitor_snapshots(core, core.bar.runtime.external_tray_width);
    let Some(snapshot) = snapshots
        .iter()
        .find(|snapshot| snapshot.monitor_id == target.monitor_id)
    else {
        return;
    };
    paint_bar_snapshot(core, x11_runtime, target, snapshot);
}

pub fn draw_bars(core: &mut CoreCtx, x11_runtime: &mut X11RuntimeConfig) {
    let targets: Vec<BarSurfaceTarget> = core
        .model()
        .monitors_iter()
        .map(|(monitor_id, monitor)| BarSurfaceTarget {
            monitor_id,
            window_id: monitor.bar_win,
        })
        .collect();
    let snapshots =
        crate::bar::scene::build_monitor_snapshots(core, core.bar.runtime.external_tray_width);
    let snapshot_by_monitor_id: HashMap<MonitorId, &crate::bar::scene::MonitorBarSnapshot> =
        snapshots
            .iter()
            .map(|snapshot| (snapshot.monitor_id, snapshot))
            .collect();

    for target in targets {
        if target.window_id == WindowId::default() {
            continue;
        }
        let Some(snapshot) = snapshot_by_monitor_id.get(&target.monitor_id).copied() else {
            continue;
        };
        paint_bar_snapshot(core, x11_runtime, target, snapshot);
    }
    core.bar.mark_drawn();
}

fn sync_monitor_bar_window(
    core: &CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    monitor: &Monitor,
    tray_monitor_id: Option<MonitorId>,
    tray_width: u32,
) {
    let bar_height = core.config().bar_metrics().height;
    let is_tray_monitor = tray_monitor_id == Some(monitor.id());

    let mut w = monitor.work_rect().w as u32;
    if core.config().systray.show && is_tray_monitor {
        w = w.saturating_sub(tray_width);
    }

    let x11_bar_win: Window = monitor.bar_win.into();
    let bounds = Rect::new(monitor.work_rect().x, monitor.bar_y(), w as i32, bar_height);
    if let Some(draw) = x11_runtime.draw.as_ref() {
        draw.queue_move_resize_window(x11_bar_win, bounds);
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

/// Commit the complete native top-bar projection.
///
/// The main bars and XEmbed tray are separate X11 windows, but callers never
/// synchronize them independently. Recommitting every monitor also releases
/// tray width from the previously selected monitor when the tray follows
/// selection.
pub fn sync_top_bar_surfaces(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    systray: &mut Option<XEmbedTray>,
) {
    let tray_monitor_id =
        crate::backend::x11::systray::systray_monitor(core.model(), &core.config().systray)
            .map(Monitor::id);
    let tray_width = if core.config().systray.show {
        crate::backend::x11::systray::get_systray_width(
            &core.config().systray,
            core.config().bar_metrics().height,
            systray.as_ref(),
        )
    } else {
        0
    };
    for (_, monitor) in core.model().monitors_iter() {
        sync_monitor_bar_window(core, x11, x11_runtime, monitor, tray_monitor_id, tray_width);
    }
    if let Some(draw) = x11_runtime.draw.as_ref() {
        draw.flush();
    }
    crate::backend::x11::systray::sync_xembed_tray(core, x11, x11_runtime, systray);
    let _ = x11.conn.flush();
    core.bar.mark_dirty();
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

fn create_missing_bar_windows(
    globals: &mut crate::core_state::CoreState,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    systray: Option<&XEmbedTray>,
) {
    let (bar_configs, xlibdisplay, root, status_bg) = {
        let bar_height = globals.config.bar_metrics().height;
        let showsystray = globals.config.systray.show;
        let status_bg: u32 = globals.config.colors.status.bg.into();
        let xlibdisplay = x11_runtime.xlibdisplay.0;
        let root = x11_runtime.root;
        let selected_monitor_id = globals.model.selected_monitor_id();

        let systray_width = if showsystray {
            crate::backend::x11::systray::get_systray_width(
                &globals.config.systray,
                bar_height,
                systray,
            )
        } else {
            0
        };

        let mut bar_configs = Vec::new();
        for (i, m) in globals.model.monitors_iter() {
            if m.bar_win != WindowId::default() {
                continue;
            }

            let mut w = m.work_rect().w as u32;
            if showsystray && selected_monitor_id == i {
                w = w.saturating_sub(systray_width);
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

    let created_top_bars = !created.is_empty();
    for (i, win_id) in created {
        if let Some(mon) = globals.model.monitor_mut(i) {
            mon.bar_win = WindowId::from(win_id);
        }
    }
    // Geometry and painting use the Xlib connection. Complete creation on the
    // XCB connection before another client connection references these IDs.
    if created_top_bars && let Ok(cookie) = conn.get_input_focus() {
        let _ = cookie.reply();
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

/// Reconcile all native bar windows with the shared monitor model.
///
/// Safe to call from any point of the topology path, including before backend
/// initialisation: a `[monitors]` config applies from `init_globals`, before
/// the DrawContext and atoms exist. Bar windows and the XEmbed tray need both,
/// so that early call is a no-op and startup reconciles again once ready.
pub fn reconcile_bar_windows(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    systray: &mut Option<XEmbedTray>,
) {
    if x11_runtime.xlibdisplay.0.is_null() || x11_runtime.draw.is_none() {
        return;
    }

    create_missing_bar_windows(core.state_mut(), x11, x11_runtime, systray.as_ref());

    sync_top_bar_surfaces(core, x11, x11_runtime, systray);
}
