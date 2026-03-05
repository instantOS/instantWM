use crate::bar::{bar_position_at_x, bar_position_to_gesture};
use crate::bar::{draw_bar, draw_bars, reset_bar};
use crate::client::{
    configure_x11, set_client_state, set_fullscreen_x11, unmanage, update_title_x11,
    update_wm_hints, WM_STATE_ICONIC, WM_STATE_WITHDRAWN,
};
use crate::contexts::{WmCtx, WmCtxX11};
// focus() is used via focus_soft() in this module
use crate::ipc::IpcServer;
use crate::keyboard::{
    grab_keys, key_press as keyboard_key_press, key_release as keyboard_key_release,
};
use crate::layouts::{arrange, restack};
use crate::monitor::update_geom;
use crate::mouse::{
    get_cursor_client_win, handle_floating_resize_hover, handle_sidebar_hover, hover_resize_mouse,
    reset_cursor, resize_mouse_directional,
};
use crate::systray;
use crate::tags::get_tag_width;
use crate::types::*;
use crate::util::clean_mask;
use crate::wm::Wm;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::CURRENT_TIME;

pub const SYSTEM_TRAY_REQUEST_DOCK: u32 = 0;

pub const XEMBED_EMBEDDED_NOTIFY: u32 = 0;
pub const XEMBED_FOCUS_IN: u32 = 4;
pub const XEMBED_WINDOW_ACTIVATE: u32 = 5;
pub const XEMBED_MODALITY_ON: u32 = 10;
pub const XEMBED_EMBEDDED_VERSION: u32 = 0;

#[inline]
fn win_to_client_ctx(ctx: &WmCtx, win: WindowId) -> Option<WindowId> {
    if ctx.g.clients.contains(&win) {
        Some(win)
    } else {
        None
    }
}

pub fn button_press(ctx: &mut WmCtxX11<'_>, e: &ButtonPressEvent) {
    let event_win = WindowId::from(e.event);
    // Client button grabs use GrabMode::SYNC; replay pointer events like dwm.
    let conn = ctx.x11.conn;
    let _ = conn.allow_events(Allow::REPLAY_POINTER, CURRENT_TIME);
    let _ = conn.flush();

    let numlockmask = ctx.core.g.numlockmask();
    let buttons_clone = ctx.core.g.cfg.buttons.clone();
    let altcursor = ctx.core.g.altcursor;
    let mut selmon_id = ctx.core.g.selected_monitor_id();
    let focusfollowsmouse = ctx.core.g.focusfollowsmouse;

    if let Some(clicked_mon) =
        ctx.core
            .g
            .monitors
            .win_to_mon(event_win, ctx.core.g.x11.root, &*ctx.core.g.clients, None)
    {
        if selmon_id != clicked_mon && (focusfollowsmouse || e.detail <= 3) {
            ctx.core.g.set_selected_monitor(clicked_mon);
            selmon_id = clicked_mon;
            crate::focus::focus_soft_x11(&mut ctx.core, &ctx.x11, None);
        }
    };

    // Determine the full bar position — this carries the exact target
    // (tag index, window handle, etc.) through to the button action.
    let bar_pos: BarPosition;

    if let Some(win) = win_to_client_ctx(&ctx.core, event_win) {
        bar_pos = BarPosition::ClientWin;
        // Only focus on button press if it's NOT a simple left/middle/right click
        // (e.g., for scroll wheel or other buttons). Simple clicks should not
        // change focus or raise windows - the user explicitly wants to interact
        // with the window without changing stacking order.
        // For focus-follows-mouse mode, we still focus since that's the expected behavior.
        if focusfollowsmouse && e.detail > 3 {
            crate::focus::focus_soft_x11(&mut ctx.core, &ctx.x11, Some(win));
            if let Some(monitor_id) = ctx.core.g.clients.get(&win).and_then(|c| c.monitor_id) {
                restack(&mut ctx.core, &ctx.backend, monitor_id);
            }
        }
    } else if let Some(mon) = ctx.core.g.monitor(selmon_id) {
        if event_win == mon.bar_win {
            let position = bar_position_at_x(mon, &ctx.core, e.event_x as i32);
            if position == BarPosition::StartMenu {
                reset_bar(&mut ctx.core, &ctx.x11);
            }
            bar_pos = position;
        } else if (e.root_x as i32) > mon.monitor_rect.x + mon.monitor_rect.w - 50 {
            bar_pos = BarPosition::SideBar;
        } else {
            bar_pos = BarPosition::Root;
        }
    } else {
        bar_pos = BarPosition::Root;
    };

    if bar_pos == BarPosition::Root {
        if let Some(mon) = ctx.core.g.monitor(selmon_id) {
            if let Some(selected_window) = mon.sel {
                let is_floating = ctx
                    .core
                    .g
                    .clients
                    .get(&selected_window)
                    .map(|c| c.isfloating)
                    .unwrap_or(false);
                let has_tiling = mon.is_tiling_layout();
                if altcursor == AltCursor::Resize && (is_floating || !has_tiling) {
                    let dir = ctx.core.g.drag.resize_direction;
                    reset_cursor(&mut ctx.core, &ctx.x11);
                    let btn = MouseButton::from_u8(e.detail).unwrap_or(MouseButton::Left);
                    if btn == MouseButton::Right {
                        crate::mouse::move_mouse_x11(&mut ctx.core, &ctx.x11, btn);
                    } else if btn == MouseButton::Left {
                        if dir == Some(crate::types::ResizeDirection::Top) {
                            crate::mouse::move_mouse_x11(&mut ctx.core, &ctx.x11, btn);
                        } else {
                            resize_mouse_directional(&mut ctx.core, &ctx.x11, dir, btn);
                        }
                    }
                    return;
                }
            }
        }
    };

    let clean_state = clean_mask(e.state.into(), numlockmask);

    for button in &buttons_clone {
        if !button.matches(bar_pos) || button.button.as_u8() != e.detail {
            continue;
        }
        if clean_mask(button.mask, numlockmask) != clean_state {
            continue;
        }
        let arg = ButtonArg {
            pos: bar_pos,
            btn: button.button,
            rx: e.root_x as i32,
            ry: e.root_y as i32,
        };
        (button.action)(ctx, arg);
    }
}

pub fn client_message(ctx: &mut WmCtx, e: &ClientMessageEvent) {
    let showsystray = ctx.g.cfg.showsystray;
    let systray_win = ctx.g.systray.as_ref().map(|s| s.win).unwrap_or_default();
    let net_system_tray_op = ctx.g.x11.netatom.system_tray_op;
    let net_wm_state = ctx.g.x11.netatom.wm_state;
    let net_active_window = ctx.g.x11.netatom.active_window;
    let event_win = WindowId::from(e.window);

    if showsystray && event_win == systray_win && e.type_ == net_system_tray_op {
        let data = e.data.as_data32();
        if data[1] == SYSTEM_TRAY_REQUEST_DOCK {
            handle_systray_dock_request(ctx, e);
        }
        return;
    };

    let Some(win) = win_to_client_ctx(ctx, event_win) else {
        return;
    };

    if e.type_ == net_wm_state {
        handle_net_wm_state(ctx, e, win);
    } else if e.type_ == net_active_window {
        handle_active_window(ctx, win);
    };
}

pub fn configure_notify(ctx: &mut WmCtx, e: &ConfigureNotifyEvent) {
    let event_win = WindowId::from(e.window);
    let root_win = WindowId::from(ctx.g.x11.root);
    if event_win != root_win {
        return;
    };

    ctx.g.cfg.screen_width = e.width as i32;
    ctx.g.cfg.screen_height = e.height as i32;

    update_geom(ctx);
    crate::focus::focus_soft(ctx, None);
    arrange(ctx, None);
}

pub fn configure_request(ctx: &mut WmCtx, e: &ConfigureRequestEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(win) = win_to_client_ctx(ctx, event_win) {
        configure(ctx, win);
    } else {
        let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
            return;
        };
        let _ = conn.configure_window(
            e.window,
            &ConfigureWindowAux::new()
                .x(e.x as i32)
                .y(e.y as i32)
                .width(e.width as u32)
                .height(e.height as u32)
                .border_width(e.border_width as u32),
        );
        let _ = conn.flush();
    };
}

pub fn create_notify(_e: &CreateNotifyEvent) {}

pub fn destroy_notify(ctx: &mut WmCtx, e: &DestroyNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(win) = win_to_client_ctx(ctx, event_win) {
        unmanage(ctx, win, true);
    } else if let Some(icon) = systray::win_to_systray_icon(ctx, event_win) {
        // Remove the icon from the systray list and client map, then resize
        // the bar and redraw the systray — matching the C code's sequence of
        // removesystrayicon(c) → resizebar_win(selmon) → updatesystray().
        systray::remove_systray_icon(ctx, icon);
        // Get monitor reference for resize_bar_win
        let selmon_idx = ctx.g.selected_monitor_id();
        if let Some(mon) = ctx.g.monitor(selmon_idx) {
            crate::bar::resize_bar_win(ctx, mon);
        }
        systray::update_systray(ctx);
    };
}

/// Handle EnterNotify events for focus-follows-mouse behavior.
///
/// This is the Rust equivalent of the C code's `enternotify` and `handle_floating_focus`.
/// The key insight is that when floating windows overlap, we must use `get_cursor_client_win`
/// (which calls XQueryPointer) to get the actual topmost window under the cursor,
/// rather than just using the event window which could be a hidden window below.
pub fn enter_notify(ctx: &mut WmCtx, e: &EnterNotifyEvent) {
    let focusfollowsmouse = ctx.g.focusfollowsmouse;
    let focusfollowsfloatmouse = ctx.g.focusfollowsfloatmouse;
    let event_win = WindowId::from(e.event);
    let entering_root = event_win == WindowId::from(ctx.g.x11.root);

    // 1. Filter out invalid crossing events (grab/ungrab, inferior notify)
    if (e.mode != NotifyMode::NORMAL || e.detail == NotifyDetail::INFERIOR) && !entering_root {
        return;
    }

    // 2. Snapshot selection state before any changes
    let selmon_id = ctx.g.selected_monitor_id();
    let selmon = ctx.g.selected_monitor();
    let selected_window = selmon.sel;
    let is_floating_sel = {
        let is_floating = selected_window
            .and_then(|w| ctx.g.clients.get(&w))
            .map(|c| c.isfloating)
            .unwrap_or(false);
        let has_tiling = selmon.is_tiling_layout();
        is_floating || !has_tiling
    };
    let entering_client = win_to_client_ctx(ctx, event_win);

    // 3. Handle floating focus (matches C handle_floating_focus)
    //    When the selected window is floating and we enter a different window
    //    (root or client), offer the resize cursor via hover_resize_mouse.
    if is_floating_sel {
        // Special case: transitioning from a floating selection to a tiled
        // client under the cursor should activate the resize offer on the
        // floating window until the user commits (clicks) or moves away.
        // This avoids the "nothing happens" feel when hovering onto a tiled
        // window while a floating window is selected.
        if crate::mouse::floating_to_tiled_hover(ctx) {
            return;
        }

        // Case 1: Entering root with floating sel
        if entering_root {
            if hover_resize_mouse(ctx) {
                return;
            }
            // Fall through to normal focus handling
        }
        // Case 2: Entering a different client while sel is floating
        else if let Some(ew) = entering_client {
            if Some(ew) != selected_window {
                let resized = hover_resize_mouse(ctx);
                if focusfollowsfloatmouse {
                    if resized {
                        return;
                    }
                    // Use the actual topmost window under cursor for focus
                    if let Some(newc) = get_cursor_client_win(ctx) {
                        if Some(newc) != selected_window {
                            crate::focus::focus_soft(ctx, Some(newc));
                        }
                    }
                }
                return;
            }
        }
    }

    // 4. Handle Monitor Switch
    if focusfollowsmouse {
        if let Some(new_mon_id) =
            ctx.g
                .monitors
                .win_to_mon(event_win, ctx.g.x11.root, &*ctx.g.clients, ctx.x11_conn())
        {
            if new_mon_id != selmon_id {
                ctx.g.set_selected_monitor(new_mon_id);
                crate::focus::focus_soft(ctx, None);
                return;
            }
        }
    }

    // 5. Determine what's actually under the cursor
    let topmost_win_under_cursor = get_cursor_client_win(ctx);

    // 6. Handle focus switching based on configuration
    crate::focus::hover_focus_target(ctx, topmost_win_under_cursor, entering_root);
}

pub fn expose(ctx: &mut WmCtx, e: &ExposeEvent) {
    if e.count != 0 {
        return;
    };

    let event_win = WindowId::from(e.window);
    if let Some(monitor_id) =
        ctx.g
            .monitors
            .win_to_mon(event_win, ctx.g.x11.root, &*ctx.g.clients, ctx.x11_conn())
    {
        let is_bar_win = ctx
            .g
            .monitors
            .get(monitor_id)
            .is_some_and(|m| event_win == m.bar_win);
        if is_bar_win {
            draw_bar(ctx, monitor_id);
        }
    };
}

pub fn focus_in(ctx: &mut WmCtx, _e: &FocusInEvent) {
    if let Some(selected_window) = ctx.selected_client() {
        crate::client::set_focus(ctx, selected_window);
    };
}

pub fn key_press(ctx: &mut WmCtx, e: &KeyPressEvent) {
    keyboard_key_press(ctx, e);
}

pub fn key_release(ctx: &mut WmCtx, e: &KeyReleaseEvent) {
    keyboard_key_release(ctx, e);
}

pub fn mapping_notify(ctx: &mut WmCtx, _e: &MappingNotifyEvent) {
    grab_keys(ctx);
}

pub fn map_request(ctx: &mut WmCtx, e: &MapRequestEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(_icon) = systray::win_to_systray_icon(ctx, event_win) {
        systray::update_systray(ctx);
        return;
    };

    if win_to_client_ctx(ctx, event_win).is_none() && !is_override_redirect(ctx, event_win) {
        let (geo, border_width) = get_win_geometry(ctx, event_win);
        crate::client::manage(ctx, event_win, geo, border_width);
    };
}

/// Handle mouse motion events for bar gesture detection and focus-follows-mouse.
pub fn motion_notify(ctx: &mut WmCtx, e: &MotionNotifyEvent) {
    let event_win = WindowId::from(e.event);
    let root_win = WindowId::from(ctx.g.x11.root);
    if event_win != root_win {
        let root_y = e.root_y as i32;
        let selmon = ctx.g.selected_monitor();
        let in_bar = selmon.showbar
            && root_y >= selmon.bar_y
            && root_y < selmon.bar_y + ctx.g.cfg.bar_height;
        if !in_bar && selmon.gesture != Gesture::None {
            reset_bar(ctx);
        }
        return;
    }

    let selmon_id = ctx.g.selected_monitor_id();
    let tagwidth = get_tag_width(ctx);

    // Update cached tag width
    ctx.g.tags.width = tagwidth;

    let root_x = e.root_x as i32;
    let root_y = e.root_y as i32;

    // Handle focus-follows-mouse monitor switching
    if ctx.g.focusfollowsmouse {
        let rect = Rect {
            x: root_x,
            y: root_y,
            w: 1,
            h: 1,
        };
        if let Some(new_mon) = crate::types::find_monitor_by_rect(ctx.g.monitors.monitors(), &rect)
            .or(Some(ctx.g.selected_monitor_id()))
        {
            if new_mon != selmon_id {
                ctx.g.set_selected_monitor(new_mon);
                crate::focus::focus_soft(ctx, None);
                return;
            }
        }
    };

    // Early-out: cursor is below the bar area.
    let (monitor_y, bar_height, current_gesture) = {
        let mon = ctx.g.selected_monitor();
        (mon.monitor_rect.y, ctx.g.cfg.bar_height, mon.gesture)
    };

    if root_y >= monitor_y + bar_height {
        if handle_floating_resize_hover(ctx, root_x, root_y, true) {
            return;
        }
        if handle_sidebar_hover(ctx, root_x, root_y) {
            return;
        }
        reset_bar(ctx);
        if ctx.g.altcursor == AltCursor::Sidebar {
            reset_cursor(ctx);
        }
        return;
    };

    // Compute the bar position from the cursor's monitor-local x coordinate,
    // then convert to a gesture for hover highlighting.
    let new_gesture = {
        let mon = ctx.g.selected_monitor();
        let local_x = root_x - mon.work_rect.x;
        let position = bar_position_at_x(mon, ctx, local_x);
        match position {
            // The status-text and root areas don't produce a hover gesture —
            // reset the bar and bail out so we don't light up anything.
            BarPosition::StatusText | BarPosition::Root => {
                reset_bar(ctx);
                return;
            }
            other => bar_position_to_gesture(other),
        }
    };

    if new_gesture != current_gesture {
        ctx.g.selected_monitor_mut().gesture = new_gesture;
        draw_bar(ctx, selmon_id);
    };
}

pub fn property_notify(ctx: &mut WmCtx, e: &PropertyNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(_icon) = systray::win_to_systray_icon(ctx, event_win) {
        systray::update_systray(ctx);
        return;
    };

    if event_win == WindowId::from(ctx.g.x11.root) && e.atom == AtomEnum::WM_NAME.into() {
        crate::bar::x11::update_status(ctx);
        return;
    };

    if let Some(win) = win_to_client_ctx(ctx, event_win) {
        match e.atom {
            x if x == AtomEnum::WM_NORMAL_HINTS.into() => {
                if let Some(c) = ctx.g.clients.get_mut(&win) {
                    c.hintsvalid = 0;
                }
            }
            x if x == AtomEnum::WM_HINTS.into() => {
                update_wm_hints(ctx, win);
                draw_bars(ctx);
            }
            _ => {}
        }

        let net_wm_name = ctx.g.x11.netatom.wm_name;
        if e.atom == AtomEnum::WM_NAME.into() || e.atom == net_wm_name {
            update_title(ctx, win);
        }
    };
}

pub fn resize_request(ctx: &mut WmCtx, e: &ResizeRequestEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(_icon) = systray::win_to_systray_icon(ctx, event_win) {
        systray::update_systray(ctx);
    };
}

pub fn unmap_notify(ctx: &mut WmCtx, e: &UnmapNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(win) = win_to_client_ctx(ctx, event_win) {
        if e.response_type & 0x80 != 0 {
            set_client_state(ctx, win, WM_STATE_WITHDRAWN);
        } else {
            unmanage(ctx, win, false);
        }
    } else if let Some(_icon) = systray::win_to_systray_icon(ctx, event_win) {
        // Systray icons sometimes unmap without destroying; re-map them.
        systray::update_systray(ctx);
    };
}

pub fn leave_notify(ctx: &mut WmCtx, _e: &LeaveNotifyEvent) {
    reset_bar(ctx);
}

fn handle_systray_dock_request(ctx: &mut WmCtx, e: &ClientMessageEvent) {
    let data = e.data.as_data32();
    let icon_win = WindowId::from(data[2]);
    if icon_win == WindowId::default() {
        return;
    };

    let selmon_id = ctx.g.selected_monitor_id();
    let systray_win_opt = ctx.g.systray.as_ref().map(|s| s.win);
    let statusescheme_bg_pixel = ctx
        .g
        .cfg
        .statusscheme
        .as_ref()
        .map(|s| s.bg.color.pixel as u32)
        .unwrap_or(0);

    let Some(systray_win) = systray_win_opt else {
        return;
    };

    let (geo, border_width) = {
        let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
            return;
        };
        let x11_icon_win: Window = icon_win.into();
        conn.get_geometry(x11_icon_win)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|wa| {
                (
                    Rect {
                        x: 0,
                        y: 0,
                        w: wa.width as i32,
                        h: wa.height as i32,
                    },
                    wa.border_width as i32,
                )
            })
            .unwrap_or((
                Rect {
                    x: 0,
                    y: 0,
                    w: 1,
                    h: 1,
                },
                0,
            ))
    };

    let client = Client {
        win: icon_win,
        geo,
        old_geo: geo,
        old_border_width: border_width,
        border_width: 0,
        isfloating: true,
        tags: 1,
        monitor_id: Some(selmon_id),
        ..Default::default()
    };

    {
        ctx.g.clients.insert(icon_win, client);
        if let Some(ref mut systray) = ctx.g.systray {
            systray.icons.insert(0, icon_win);
        }
    };

    crate::client::update_size_hints(ctx, icon_win);
    systray::update_systray_icon_geom(ctx, icon_win, geo.w, geo.h);

    if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
        let x11_icon_win: Window = icon_win.into();
        let x11_systray_win: Window = systray_win.into();

        let _ = conn.change_save_set(SetMode::INSERT, x11_icon_win);

        let mask =
            EventMask::STRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE | EventMask::RESIZE_REDIRECT;
        let _ = conn.change_window_attributes(
            x11_icon_win,
            &ChangeWindowAttributesAux::new().event_mask(mask),
        );

        let _ = conn.reparent_window(x11_icon_win, x11_systray_win, 0, 0);

        let _ = conn.change_window_attributes(
            x11_icon_win,
            &ChangeWindowAttributesAux::new().background_pixel(statusescheme_bg_pixel),
        );

        let _ = conn.flush();
    }

    let xembed_atom = ctx.g.x11.xatom.xembed;
    let structure_notify_mask = EventMask::STRUCTURE_NOTIFY.bits();

    crate::client::send_event(
        ctx,
        icon_win,
        xembed_atom,
        structure_notify_mask,
        CURRENT_TIME as i64,
        XEMBED_EMBEDDED_NOTIFY as i64,
        0,
        u32::from(systray_win) as i64,
        XEMBED_EMBEDDED_VERSION as i64,
    );
    crate::client::send_event(
        ctx,
        icon_win,
        xembed_atom,
        structure_notify_mask,
        CURRENT_TIME as i64,
        XEMBED_FOCUS_IN as i64,
        0,
        u32::from(systray_win) as i64,
        XEMBED_EMBEDDED_VERSION as i64,
    );
    crate::client::send_event(
        ctx,
        icon_win,
        xembed_atom,
        structure_notify_mask,
        CURRENT_TIME as i64,
        XEMBED_WINDOW_ACTIVATE as i64,
        0,
        u32::from(systray_win) as i64,
        XEMBED_EMBEDDED_VERSION as i64,
    );
    crate::client::send_event(
        ctx,
        icon_win,
        xembed_atom,
        structure_notify_mask,
        CURRENT_TIME as i64,
        XEMBED_MODALITY_ON as i64,
        0,
        u32::from(systray_win) as i64,
        XEMBED_EMBEDDED_VERSION as i64,
    );

    if let Some(mon) = ctx.g.monitor(selmon_id) {
        crate::bar::resize_bar_win(ctx, mon);
    };

    systray::update_systray(ctx);
    set_client_state(ctx, icon_win, 1);
}

fn handle_net_wm_state(ctx: &mut WmCtx, e: &ClientMessageEvent, win: WindowId) {
    let data = e.data.as_data32();
    let fullscreen_action = data[0];

    if fullscreen_action == 1 {
        set_fullscreen(ctx, win, true);
    } else if fullscreen_action == 0 {
        set_fullscreen(ctx, win, false);
    };
}

fn handle_active_window(ctx: &mut WmCtx, win: WindowId) {
    let is_hidden = ctx.g.clients.is_hidden(win);
    if is_hidden {
        crate::client::show(ctx, win);
    };

    if let Some(c) = ctx.g.clients.get(&win) {
        if let Some(monitor_id) = c.monitor_id {
            crate::focus::focus_soft(ctx, Some(win));
            restack(ctx, monitor_id);
        }
    };
}

pub fn run(wm: &mut Wm, ipc_server: &mut Option<IpcServer>) {
    use std::os::unix::io::AsRawFd;

    // Pre-fetch the X11 connection file descriptor for poll(2).
    let x11_fd = wm
        .backend
        .x11()
        .map(|x11| x11.conn.stream().as_raw_fd())
        .unwrap_or(-1);
    let ipc_fd = ipc_server.as_ref().map(|s| s.as_raw_fd()).unwrap_or(-1);

    while wm.running {
        // ── 1. Drain all pending X11 events ─────────────────────────────
        let mut handled = false;
        loop {
            let event = wm
                .backend
                .x11()
                .and_then(|x11| x11.conn.poll_for_event().ok())
                .flatten();
            match event {
                Some(event) => {
                    dispatch_event(wm, event);
                    handled = true;
                }
                None => break,
            }
        }

        // ── 2. Process any pending IPC commands ─────────────────────────
        if let Some(server) = ipc_server.as_mut() {
            server.process_pending(wm);
        }

        // ── 3. Wait for new data on X11 fd and/or IPC fd ────────────────
        // Skip the wait when we just handled events — there may be more
        // events that arrived while we were dispatching.
        if !handled {
            let mut fds = [
                libc::pollfd {
                    fd: x11_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: ipc_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let nfds = if ipc_fd >= 0 { 2 } else { 1 };
            // Block until data arrives (or 100ms timeout as safety net).
            unsafe {
                libc::poll(fds.as_mut_ptr(), nfds as libc::nfds_t, 100);
            }
        }
    }
}

fn dispatch_event(wm: &mut Wm, event: x11rb::protocol::Event) {
    let mut ctx = wm.ctx();

    match event {
        x11rb::protocol::Event::ButtonPress(e) => button_press(&mut ctx, &e),
        x11rb::protocol::Event::ClientMessage(e) => client_message(&mut ctx, &e),
        x11rb::protocol::Event::ConfigureNotify(e) => configure_notify(&mut ctx, &e),
        x11rb::protocol::Event::ConfigureRequest(e) => configure_request(&mut ctx, &e),
        x11rb::protocol::Event::CreateNotify(e) => create_notify(&e),
        x11rb::protocol::Event::DestroyNotify(e) => destroy_notify(&mut ctx, &e),
        x11rb::protocol::Event::EnterNotify(e) => enter_notify(&mut ctx, &e),
        x11rb::protocol::Event::Expose(e) => expose(&mut ctx, &e),
        x11rb::protocol::Event::FocusIn(e) => focus_in(&mut ctx, &e),
        x11rb::protocol::Event::KeyPress(e) => key_press(&mut ctx, &e),
        x11rb::protocol::Event::KeyRelease(e) => key_release(&mut ctx, &e),
        x11rb::protocol::Event::MappingNotify(e) => mapping_notify(&mut ctx, &e),
        x11rb::protocol::Event::MapRequest(e) => map_request(&mut ctx, &e),
        x11rb::protocol::Event::MotionNotify(e) => motion_notify(&mut ctx, &e),
        x11rb::protocol::Event::PropertyNotify(e) => property_notify(&mut ctx, &e),
        x11rb::protocol::Event::ResizeRequest(e) => resize_request(&mut ctx, &e),
        x11rb::protocol::Event::UnmapNotify(e) => unmap_notify(&mut ctx, &e),
        x11rb::protocol::Event::LeaveNotify(e) => leave_notify(&mut ctx, &e),
        _ => {}
    };
}

// ---------------------------------------------------------------------------
// scan helpers
// ---------------------------------------------------------------------------

/// Fetch the geometry and border width for `win`.
///
/// Returns a fallback (`800×600`, border `1`) when the request fails.
fn get_win_geometry(ctx: &WmCtx, win: WindowId) -> (Rect, u32) {
    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return (
            Rect {
                x: 0,
                y: 0,
                w: 800,
                h: 600,
            },
            1,
        );
    };
    let x11_win: Window = win.into();
    conn.get_geometry(x11_win)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|geo| {
            (
                Rect {
                    x: geo.x as i32,
                    y: geo.y as i32,
                    w: geo.width as i32,
                    h: geo.height as i32,
                },
                geo.border_width as u32,
            )
        })
        .unwrap_or((
            Rect {
                x: 0,
                y: 0,
                w: 800,
                h: 600,
            },
            1,
        ))
}

/// Returns `true` when the `override_redirect` attribute is set on `win`.
fn is_override_redirect(ctx: &WmCtx, win: WindowId) -> bool {
    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return false;
    };
    let x11_win: Window = win.into();
    conn.get_window_attributes(x11_win)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|wa| wa.override_redirect)
        .unwrap_or(false)
}

/// Partition `children` into `(managed, transients)`.
fn classify_windows(ctx: &WmCtx, children: Vec<Window>) -> (Vec<WindowId>, Vec<WindowId>) {
    let mut managed = Vec::new();
    let mut transients = Vec::new();

    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return (managed, transients);
    };

    for win in children {
        let win_id = WindowId::from(win);
        if is_override_redirect(ctx, win_id) {
            continue;
        }

        // Skip windows that are neither visible nor iconic.
        let is_viewable = conn
            .get_window_attributes(win)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|wa| wa.map_state == MapState::VIEWABLE)
            .unwrap_or(false);
        let is_iconic = is_window_iconic(ctx, win_id);

        if !is_viewable && !is_iconic {
            continue;
        }

        // Skip already-managed windows.
        if win_to_client_ctx(ctx, win_id).is_some() {
            continue;
        }

        // Check WM_TRANSIENT_FOR directly using the already-borrowed conn.
        let is_transient = conn
            .get_property(
                false,
                win,
                AtomEnum::WM_TRANSIENT_FOR,
                AtomEnum::WINDOW,
                0,
                1,
            )
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().and_then(|mut it| it.next()))
            .is_some();
        if is_transient {
            transients.push(win_id);
        } else {
            managed.push(win_id);
        }
    }

    (managed, transients)
}

fn is_window_iconic(ctx: &WmCtx, win: WindowId) -> bool {
    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return false;
    };
    let x11_win: Window = win.into();

    let state_atom = ctx.g.x11.wmatom.state;
    let Ok(cookie) = conn.get_property(false, x11_win, state_atom, state_atom, 0, 2) else {
        return false;
    };
    let Ok(reply) = cookie.reply() else {
        return false;
    };

    reply
        .value32()
        .and_then(|mut it| it.next())
        .map(|v| v as i32 == WM_STATE_ICONIC)
        .unwrap_or(false)
}

/// Adopt all pre-existing X11 windows at WM startup.
pub fn scan(wm: &mut Wm) {
    let mut ctx = wm.ctx();
    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return;
    };
    let root = ctx.g.x11.root;

    let children = {
        let Ok(tree_cookie) = conn.query_tree(root) else {
            return;
        };
        let Ok(tree_reply) = tree_cookie.reply() else {
            return;
        };
        tree_reply.children
    };

    let (managed, transients) = classify_windows(&ctx, children);

    for win in managed.into_iter().chain(transients) {
        let (geo, border_width) = get_win_geometry(&ctx, win);
        crate::client::manage(&mut ctx, win, geo, border_width);
    }
}

pub fn check_other_wm(conn: &x11rb::rust_connection::RustConnection, root: Window) {
    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    let _ = conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(mask));
}

pub fn setup(_wm: &mut Wm) {}

pub fn setup_root(wm: &mut Wm) {
    let Some(x11) = wm.backend.x11() else {
        return;
    };
    let conn = &x11.conn;
    let root = wm.g.x11.root;
    let mask = EventMask::SUBSTRUCTURE_REDIRECT
        | EventMask::SUBSTRUCTURE_NOTIFY
        | EventMask::BUTTON_PRESS
        | EventMask::POINTER_MOTION
        | EventMask::ENTER_WINDOW
        | EventMask::LEAVE_WINDOW
        | EventMask::STRUCTURE_NOTIFY
        | EventMask::PROPERTY_CHANGE
        | EventMask::KEY_PRESS
        | EventMask::KEY_RELEASE;

    let _ = conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(mask));
    let _ = conn.flush();

    let mut ctx = wm.ctx();
    update_geom(&mut ctx);
}

pub fn cleanup(wm: &mut Wm) {
    let Some(x11) = wm.backend.x11() else {
        return;
    };
    let conn = &x11.conn;

    let _ = conn.grab_server();

    for (_id, mon) in wm.g.monitors_iter() {
        for &win in &mon.clients {
            if let Some(c) = wm.g.clients.get(&win) {
                let old_bw = c.old_border_width;
                let x11_win: Window = win.into();
                let _ = conn.configure_window(
                    x11_win,
                    &ConfigureWindowAux::new().border_width(old_bw as u32),
                );
            }
        }
    }

    let _ = conn.ungrab_server();
    let _ = conn.flush();
}
