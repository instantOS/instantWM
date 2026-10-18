use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::core_state::WmCore;
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

/// Last submitted bottom-strip state, keyed by native window identity.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct BottomBarProjection {
    bounds: Rect,
    indicator_window: WindowId,
    indicator: Rect,
    background: u32,
}

fn paint_bar_snapshot(
    core: &mut WmCore,
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

fn draw_bars(core: &mut WmCore, x11_runtime: &mut X11RuntimeConfig) {
    let targets: Vec<BarSurfaceTarget> = core
        .state
        .model
        .monitors_iter()
        .map(|(monitor_id, monitor)| BarSurfaceTarget {
            monitor_id,
            window_id: monitor.bar_win,
        })
        .collect();
    let snapshots = crate::bar::scene::build_monitor_snapshots(
        core,
        core.bar.runtime.external_tray_width,
        crate::bar::scene::BarSnapshotScope::Visible,
    );
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
    core: &WmCore,
    x11_runtime: &X11RuntimeConfig,
    monitor: &Monitor,
    tray_monitor_id: Option<MonitorId>,
    tray_width: u32,
) {
    let bar_height = core.state.config.bar_metrics().height;
    let is_tray_monitor = tray_monitor_id == Some(monitor.id());

    let mut w = monitor.work_rect().w as u32;
    if core.state.config.systray.show && is_tray_monitor {
        w = w.saturating_sub(tray_width);
    }

    let x11_bar_win: Window = monitor.bar_win.into();
    let bounds = Rect::new(monitor.work_rect().x, monitor.bar_y(), w as i32, bar_height);
    if let Some(draw) = x11_runtime.draw.as_ref() {
        draw.present_window(x11_bar_win, bounds);
    }
}

/// Commit the complete native top-bar projection.
///
/// The main bars and XEmbed tray are separate X11 windows, but callers never
/// synchronize them independently. Recommitting every monitor also releases
/// tray width from the previously selected monitor when the tray follows
/// selection.
fn sync_top_bar_surfaces(
    core: &mut WmCore,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    systray: &mut Option<XEmbedTray>,
) {
    let tray_monitor_id =
        crate::systray::monitor(&core.state.model, &core.state.config.systray).map(Monitor::id);
    let tray_width = if core.state.config.systray.show {
        crate::backend::x11::systray::get_systray_width(
            &core.state.config.systray,
            core.state.config.bar_metrics().height,
            systray.as_ref(),
        )
    } else {
        0
    };
    for (_, monitor) in core.state.model.monitors_iter() {
        sync_monitor_bar_window(core, x11_runtime, monitor, tray_monitor_id, tray_width);
    }
    if let Some(draw) = x11_runtime.draw.as_ref() {
        draw.flush();
    }
    crate::backend::x11::systray::sync_xembed_tray(core, x11, x11_runtime, systray);
    let _ = x11.conn.flush();
}

/// Move/resize the bottom bar window to its current monitor strip and re-apply
/// its background color. `m.bottom_bar_y()` slides the window off-screen when
/// the bar is hidden, so this doubles as the show/hide operation.
fn resize_bottom_bar_win(globals: &crate::core_state::CoreState, x11: &X11BackendRef, m: &Monitor) {
    let bottom_win: Window = m.bottom_bar_win.into();
    if bottom_win == 0 {
        return;
    }
    let status_bg: u32 = globals.config.colors.status.background.into();
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
    let _ = x11.conn.map_window(bottom_win);
}

/// Allocate an unmapped surface. Its geometry and visibility are applied by
/// synchronization, so creation never duplicates the presentation policy.
fn create_bar_window(
    x11: &X11BackendRef,
    parent: Window,
    background: u32,
    events: x11rb::protocol::xproto::EventMask,
) -> WindowId {
    use x11rb::protocol::xproto::{CreateWindowAux, WindowClass};
    let window = x11
        .conn
        .generate_id()
        .expect("failed to generate bar window ID");
    let _ = x11.conn.create_window(
        x11rb::COPY_FROM_PARENT as u8,
        window,
        parent,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new()
            .override_redirect(1)
            .background_pixel(background)
            .event_mask(events),
    );
    window.into()
}

fn create_missing_bar_windows(
    state: &mut crate::core_state::CoreState,
    x11: &X11BackendRef,
    root: Window,
) -> bool {
    use x11rb::protocol::xproto::EventMask;
    let background = state.config.colors.status.background.into();
    let mut created_top_bars = false;
    let mut created_bottom_bars = false;
    for (_, monitor) in state.model.monitors_iter_mut() {
        if monitor.bar_win == WindowId::default() {
            monitor.bar_win = create_bar_window(
                x11,
                root,
                background,
                EventMask::BUTTON_PRESS | EventMask::EXPOSURE | EventMask::LEAVE_WINDOW,
            );
            created_top_bars = true;
        }
        if monitor.bottom_bar_win == WindowId::default() {
            created_bottom_bars = true;
            // Bottom strip presses propagate to the root for WM hit testing.
            monitor.bottom_bar_win = create_bar_window(x11, root, background, EventMask::NO_EVENT);
            monitor.bottom_bar_indicator_win = create_bar_window(
                x11,
                monitor.bottom_bar_win.into(),
                0xffffff,
                EventMask::NO_EVENT,
            );
            let _ = x11.conn.map_window(monitor.bottom_bar_indicator_win.into());
        }
    }
    // Painting uses a separate Xlib connection. Confirm that the server knows
    // the new target IDs before submitting Xlib drawing requests.
    if created_top_bars && let Ok(cookie) = x11.conn.get_input_focus() {
        let _ = cookie.reply();
    }
    created_top_bars || created_bottom_bars
}

/// Reconcile all native bar windows with the shared monitor model.
fn reconcile_bar_windows(
    core: &mut WmCore,
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
    systray: &mut Option<XEmbedTray>,
) -> bool {
    let created = create_missing_bar_windows(&mut core.state, x11, x11_runtime.root);
    for (_, monitor) in core.state.model.monitors_iter() {
        let projection = BottomBarProjection {
            bounds: Rect::new(
                monitor.work_rect().x,
                monitor.bottom_bar_y(),
                monitor.work_rect().w,
                monitor.bottom_bar_height,
            ),
            indicator_window: monitor.bottom_bar_indicator_win,
            indicator: monitor.bottom_bar_indicator_rect(),
            background: core.state.config.colors.status.background.into(),
        };
        if x11_runtime
            .bottom_bar_projections
            .get(&monitor.bottom_bar_win)
            != Some(&projection)
        {
            resize_bottom_bar_win(&core.state, x11, monitor);
            x11_runtime
                .bottom_bar_projections
                .insert(monitor.bottom_bar_win, projection);
        }
    }

    x11_runtime.bottom_bar_projections.retain(|window, _| {
        core.state
            .model
            .monitors_iter()
            .any(|(_, monitor)| monitor.bottom_bar_win == *window)
    });
    sync_top_bar_surfaces(core, x11, x11_runtime, systray);
    created
}

/// The only X11 bar projection boundary. Appearance also owns native border
/// colors/cursors in the drawing context, so refresh existing client borders
/// after replacing it, even if their layout and focus did not change.
pub fn reconcile_bars(ctx: &mut crate::contexts::WmCtxX11<'_>) {
    use crate::bar::BarChange;
    // Policy can change before startup has initialized the drawing connection.
    // Do not consume work until the native resources are ready.
    if ctx.x11_runtime.xlibdisplay.0.is_null() || ctx.x11_runtime.draw.is_none() {
        return;
    }
    let Some(change) = ctx.core.bar.take_pending_change() else {
        return;
    };
    if change == BarChange::Appearance {
        crate::backend::x11::startup::init_drw_and_schemes_impl(
            ctx.x11_runtime,
            &ctx.core.state.config,
        );
        let selected = ctx.core.state.model.selected_win();
        for (_, client) in ctx.core.state.model.clients_iter_all() {
            crate::backend::x11::focus::refresh_border_color(
                &ctx.core.state,
                &ctx.x11,
                ctx.x11_runtime,
                client.win,
                selected == Some(client.win),
            );
        }
    }
    if change >= BarChange::Presentation {
        let created = reconcile_bar_windows(ctx.core, &ctx.x11, ctx.x11_runtime, ctx.xembed_tray);
        // Newly created windows were absent from earlier stacking projections.
        // Surface/tray synchronization must finish before restoring WM layers.
        if created {
            ctx.x11
                .apply_z_order(&crate::layouts::global_z_order(&ctx.core.state.model));
        }
        let _ = ctx.x11.conn.flush();
    }
    draw_bars(ctx.core, ctx.x11_runtime);
}

#[cfg(test)]
mod tests {
    use crate::bar::BarChange;
    use crate::runtime::{TickOptions, event_loop_tick_with_options};
    use crate::test_support::MonitorBuilder;
    use crate::types::{Rect, WindowId};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;

    fn geometry(
        wm: &crate::wm::X11Wm,
        window: WindowId,
    ) -> x11rb::protocol::xproto::GetGeometryReply {
        // Geometry is submitted asynchronously on the painting connection.
        // Complete it before observing the server from the x11rb connection.
        unsafe extern "C" {
            fn XSync(display: *mut libc::c_void, discard: libc::c_int) -> libc::c_int;
        }
        unsafe {
            XSync(wm.backend.x11_runtime.xlibdisplay.0, 0);
        }
        wm.backend
            .conn
            .get_geometry(window.into())
            .unwrap()
            .reply()
            .unwrap()
    }

    /// Uses real server geometry to catch missed reconciliation on an empty
    /// desktop, including deferred resource initialization and bottom strips.
    #[test]
    #[ignore = "requires a dedicated Xvfb display"]
    fn empty_desktop_projects_bar_changes_at_tick_boundary() {
        let (conn, screen) = x11rb::connect(None).expect("test requires Xvfb");
        let root = conn.setup().roots[screen].root;
        let mut wm = crate::wm::X11Wm::new(crate::backend::X11BackendData::new(conn, screen));
        wm.backend.x11_runtime.root = root;
        wm.core.state.config.systray.show = false;
        let monitor = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .monitor_rect(Rect::new(0, 0, 800, 600))
                .bottom_bar(12, true)
                .build(),
        );
        wm.core.state.model.monitors.set_selected(monitor);
        crate::monitor::resync_monitor_ui_metrics(&mut wm.core.state);
        wm.core.bar.invalidate(BarChange::Presentation);
        // Early policy changes survive until native startup is ready.
        wm.x11_ctx().reconcile_bars();
        assert_eq!(
            wm.core.state.model.monitor(monitor).unwrap().bar_win,
            WindowId::default()
        );
        crate::backend::x11::startup::init_drw_and_schemes_impl(
            &mut wm.backend.x11_runtime,
            &wm.core.state.config,
        );
        let tick = |wm: &mut crate::wm::X11Wm| {
            event_loop_tick_with_options(&mut wm.x11_ctx(), &mut None, TickOptions::default());
        };
        tick(&mut wm);
        let top = wm.core.state.model.monitor(monitor).unwrap().bar_win;
        let bottom = wm.core.state.model.monitor(monitor).unwrap().bottom_bar_win;
        assert_ne!(top, WindowId::default());
        assert_ne!(bottom, WindowId::default());
        assert!(!wm.core.bar.needs_redraw());
        assert_eq!(
            wm.core.bar.take_pending_change(),
            None,
            "projection must not invalidate itself"
        );

        crate::toggles::toggle_bar(&mut wm.x11_ctx());
        crate::toggles::set_bottom_bar_shown(&mut wm.x11_ctx(), false);
        // Before the tick, the old native geometry is still visible.
        let before = geometry(&wm, top);
        assert!(before.y >= 0);
        let result =
            event_loop_tick_with_options(&mut wm.x11_ctx(), &mut None, TickOptions::default());
        assert!(!result.layout_applied, "there are no clients to arrange");
        let top_geo = geometry(&wm, top);
        let bottom_geo = geometry(&wm, bottom);
        assert!(top_geo.y < 0);
        assert!(bottom_geo.y >= 600);

        // An appearance change updates geometry without replacing bar windows.
        wm.core.state.config.fonts.text_size = 32.0;
        wm.core.state.config.bar.height = 48;
        crate::monitor::resync_monitor_ui_metrics(&mut wm.core.state);
        wm.core.bar.invalidate(BarChange::Appearance);
        tick(&mut wm);
        assert_eq!(wm.core.state.model.monitor(monitor).unwrap().bar_win, top);
        assert_eq!(geometry(&wm, top).height, 48);
        assert!(!wm.core.bar.needs_redraw());
        assert_eq!(wm.core.bar.take_pending_change(), None);

        // A bar created at the tick boundary must respect existing WM layers.
        let new_monitor = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .monitor_rect(Rect::new(800, 0, 800, 600))
                .tag_count(1)
                .build(),
        );
        let floating = wm.backend.conn.generate_id().unwrap();
        wm.backend
            .conn
            .create_window(
                x11rb::COPY_FROM_PARENT as u8,
                floating,
                root,
                850,
                50,
                200,
                100,
                0,
                x11rb::protocol::xproto::WindowClass::INPUT_OUTPUT,
                0,
                &x11rb::protocol::xproto::CreateWindowAux::new(),
            )
            .unwrap()
            .check()
            .unwrap();
        wm.backend
            .conn
            .map_window(floating)
            .unwrap()
            .check()
            .unwrap();
        let mut client = crate::types::Client {
            win: floating.into(),
            tags: crate::types::TagMask::from_bits(1),
            ..Default::default()
        };
        client.set_placement(crate::types::ClientPlacement::Floating);
        crate::test_support::add_client(&mut wm.core.state.model, new_monitor, client);
        wm.core.bar.invalidate(BarChange::Presentation);
        wm.x11_ctx().reconcile_bars();
        let new_bar = wm.core.state.model.monitor(new_monitor).unwrap().bar_win;
        let children = wm
            .backend
            .conn
            .query_tree(root)
            .unwrap()
            .reply()
            .unwrap()
            .children;
        let bar_index = children
            .iter()
            .position(|id| *id == u32::from(new_bar))
            .unwrap();
        let floating_index = children.iter().position(|id| *id == floating).unwrap();
        assert!(
            bar_index < floating_index,
            "new bars belong below floating clients"
        );

        // An unchanged presentation must not configure bottom strips or
        // restack clients that the focus/layout path already positioned.
        for window in [floating, u32::from(bottom)] {
            wm.backend
                .conn
                .change_window_attributes(
                    window,
                    &x11rb::protocol::xproto::ChangeWindowAttributesAux::new()
                        .event_mask(x11rb::protocol::xproto::EventMask::STRUCTURE_NOTIFY),
                )
                .unwrap()
                .check()
                .unwrap();
        }
        while wm.backend.conn.poll_for_event().unwrap().is_some() {}
        wm.core.bar.invalidate(BarChange::Presentation);
        wm.x11_ctx().reconcile_bars();
        wm.backend.conn.get_input_focus().unwrap().reply().unwrap();
        while let Some(event) = wm.backend.conn.poll_for_event().unwrap() {
            if let x11rb::protocol::Event::ConfigureNotify(event) = event {
                assert!(
                    event.window != floating && event.window != u32::from(bottom),
                    "unchanged presentation should not configure existing clients or bottom strips"
                );
            }
        }
    }
}
