use crate::backend::x11::lifecycle::{manage, unmanage};
use crate::backend::x11::X11BackendRef;
use crate::backend::BackendOps;
use crate::bar::{bar_position_at_x, bar_position_to_gesture};
use crate::bar::{draw_bar, draw_bars_x11, reset_bar_x11};
use crate::client::{
    configure_x11, set_client_state, set_fullscreen_x11, update_title_x11, update_wm_hints,
    WM_STATE_ICONIC, WM_STATE_WITHDRAWN,
};
use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
use crate::ipc::IpcServer;
use crate::keyboard::{grab_keys_x11, key_press_x11, key_release_x11};
use crate::layouts::{arrange, restack};
use crate::monitor::update_geom;
use crate::mouse::{
    handle_floating_resize_hover, handle_sidebar_hover, hover_resize_mouse, reset_cursor_x11,
    resize_mouse_directional,
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
fn win_to_client_ctx(core: &crate::contexts::CoreCtx, win: WindowId) -> Option<WindowId> {
    if core.g.clients.contains(&win) {
        Some(win)
    } else {
        None
    }
}

fn button_press_x11(ctx: &mut WmCtxX11<'_>, e: &ButtonPressEvent) {
    let event_win = WindowId::from(e.event);
    // Client button grabs use GrabMode::SYNC; replay pointer events like dwm.
    let conn = ctx.x11.conn;
    let _ = conn.allow_events(Allow::REPLAY_POINTER, CURRENT_TIME);
    let _ = conn.flush();

    let numlockmask = ctx.x11_runtime().numlockmask;
    let buttons_clone = ctx.core.g.cfg.buttons.clone();
    let altcursor = ctx.core.g.altcursor;
    let mut selmon_id = ctx.core.g.selected_monitor_id();
    let focusfollowsmouse = ctx.core.g.focusfollowsmouse;

    if let Some(clicked_mon) = ctx.core.g.monitors.win_to_mon(
        event_win,
        ctx.x11_runtime().root,
        &*ctx.core.g.clients,
        None,
    ) {
        if selmon_id != clicked_mon && (focusfollowsmouse || e.detail <= 3) {
            ctx.core.g.set_selected_monitor(clicked_mon);
            selmon_id = clicked_mon;
            crate::focus::focus_soft_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, None);
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
            crate::focus::focus_soft_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, Some(win));
            if let Some(monitor_id) = ctx.core.g.clients.get(&win).and_then(|c| c.monitor_id) {
                restack(&mut WmCtx::X11(ctx.reborrow()), monitor_id);
            }
        }
    } else if let Some(mon) = ctx.core.g.monitor(selmon_id) {
        if event_win == mon.bar_win {
            let position = bar_position_at_x(mon, &ctx.core, e.event_x as i32);
            if position == BarPosition::StartMenu {
                reset_bar_x11(
                    &mut ctx.core,
                    &ctx.x11,
                    ctx.x11_runtime,
                    ctx.systray.as_deref(),
                );
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
                    reset_cursor_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime);
                    let btn = MouseButton::from_u8(e.detail).unwrap_or(MouseButton::Left);
                    let mut x11_ctx = ctx.reborrow();
                    if btn == MouseButton::Right {
                        crate::mouse::move_mouse(&mut x11_ctx, btn);
                    } else if btn == MouseButton::Left {
                        if dir == Some(crate::types::ResizeDirection::Top) {
                            crate::mouse::move_mouse(&mut x11_ctx, btn);
                        } else {
                            resize_mouse_directional(&mut x11_ctx, dir, btn);
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
        // Convert WmCtxX11 to WmCtx for button action
        let tmp = ctx.reborrow();
        (button.action)(&mut WmCtx::X11(tmp), arg);
    }
}

pub fn client_message(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent) {
    let showsystray = ctx.core.g.cfg.showsystray;
    let systray_win = ctx.systray.as_ref().map(|s| s.win).unwrap_or_default();
    let net_system_tray_op = ctx.x11_runtime.netatom.system_tray_op;
    let net_wm_state = ctx.x11_runtime.netatom.wm_state;
    let net_active_window = ctx.x11_runtime.netatom.active_window;
    let event_win = WindowId::from(e.window);

    if showsystray && event_win == systray_win && e.type_ == net_system_tray_op {
        let data = e.data.as_data32();
        if data[1] == SYSTEM_TRAY_REQUEST_DOCK {
            handle_systray_dock_request(ctx, e);
        }
        return;
    };

    let Some(win) = win_to_client_ctx(&ctx.core, event_win) else {
        return;
    };

    if e.type_ == net_wm_state {
        handle_net_wm_state(ctx, e, win);
    } else if e.type_ == net_active_window {
        handle_active_window(ctx, win);
    };
}

pub fn configure_notify(ctx: &mut WmCtxX11<'_>, e: &ConfigureNotifyEvent) {
    let event_win = WindowId::from(e.window);
    let root_win = WindowId::from(ctx.x11_runtime.root);
    if event_win != root_win {
        return;
    };

    ctx.core.g.cfg.screen_width = e.width as i32;
    ctx.core.g.cfg.screen_height = e.height as i32;

    update_geom(&mut WmCtx::X11(ctx.reborrow()));
    crate::focus::focus_soft_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, None);
    arrange(&mut WmCtx::X11(ctx.reborrow()), None);
}

pub fn configure_request(ctx: &mut WmCtxX11<'_>, e: &ConfigureRequestEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(win) = win_to_client_ctx(&ctx.core, event_win) {
        configure_x11(&mut ctx.core, &ctx.x11, win);
    } else {
        let conn = ctx.x11.conn;
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

pub fn destroy_notify(ctx: &mut WmCtxX11<'_>, e: &DestroyNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(win) = win_to_client_ctx(&ctx.core, event_win) {
        let mut tmp = ctx.reborrow();
        unmanage(&mut tmp, win, true);
    } else if let Some(icon) =
        systray::win_to_systray_icon(&ctx.core, ctx.systray.as_deref(), event_win)
    {
        // Remove the icon from the systray list and client map, then resize
        // the bar and redraw the systray — matching the C code's sequence of
        // removesystrayicon(c) → resizebar_win(selmon) → updatesystray().
        systray::remove_systray_icon(&mut ctx.core, ctx.systray.as_deref_mut(), icon);
        // Get monitor reference for resize_bar_win
        let selmon_idx = ctx.core.g.selected_monitor_id();
        if let Some(mon) = ctx.core.g.monitor(selmon_idx).cloned() {
            crate::bar::resize_bar_win(
                &ctx.core,
                &ctx.x11,
                ctx.x11_runtime,
                ctx.systray.as_deref(),
                &mon,
            );
        }
        systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref_mut(),
        );
    };
}

/// Handle EnterNotify events for focus-follows-mouse behavior.
///
/// This is the Rust equivalent of the C code's `enternotify` and `handle_floating_focus`.
/// The key insight is that when floating windows overlap, we must use `get_cursor_client_win`
/// (which calls XQueryPointer) to get the actual topmost window under the cursor,
/// rather than just using the event window which could be a hidden window below.
pub fn enter_notify(ctx: &mut WmCtxX11<'_>, e: &EnterNotifyEvent) {
    let focusfollowsmouse = ctx.core.g.focusfollowsmouse;
    let focusfollowsfloatmouse = ctx.core.g.focusfollowsfloatmouse;
    let event_win = WindowId::from(e.event);
    let entering_root = event_win == WindowId::from(ctx.x11_runtime.root);

    // 1. Filter out invalid crossing events (grab/ungrab, inferior notify)
    if (e.mode != NotifyMode::NORMAL || e.detail == NotifyDetail::INFERIOR) && !entering_root {
        return;
    }

    // 2. Snapshot selection state before any changes
    let selmon_id = ctx.core.g.selected_monitor_id();
    let selmon = ctx.core.g.selected_monitor();
    let selected_window = selmon.sel;
    let is_floating_sel = {
        let is_floating = selected_window
            .and_then(|w| ctx.core.g.clients.get(&w))
            .map(|c| c.isfloating)
            .unwrap_or(false);
        let has_tiling = selmon.is_tiling_layout();
        is_floating || !has_tiling
    };
    let entering_client = win_to_client_ctx(&ctx.core, event_win);

    // 3. Handle floating focus (matches C handle_floating_focus)
    //    When the selected window is floating and we enter a different window
    //    (root or client), offer the resize cursor via hover_resize_mouse.
    if is_floating_sel {
        // Special case: transitioning from a floating selection to a tiled
        // client under the cursor should activate the resize offer on the
        // floating window until the user commits (clicks) or moves away.
        // This avoids the "nothing happens" feel when hovering onto a tiled
        // window while a floating window is selected.
        if crate::mouse::floating_to_tiled_hover(&mut WmCtx::X11(ctx.reborrow())) {
            return;
        }

        // Case 1: Entering root with floating sel
        if entering_root {
            if hover_resize_mouse(&mut WmCtx::X11(ctx.reborrow())) {
                return;
            }
            // Fall through to normal focus handling
        }
        // Case 2: Entering a different client while sel is floating
        else if let Some(ew) = entering_client {
            if Some(ew) != selected_window {
                let resized = hover_resize_mouse(&mut WmCtx::X11(ctx.reborrow()));
                if focusfollowsfloatmouse {
                    if resized {
                        return;
                    }
                    // Use the actual topmost window under cursor for focus
                    if let Some(newc) = crate::mouse::get_cursor_client_win_x11(
                        &ctx.core,
                        &ctx.x11,
                        ctx.x11_runtime,
                    ) {
                        if Some(newc) != selected_window {
                            crate::focus::focus_soft_x11(
                                &mut ctx.core,
                                &ctx.x11,
                                ctx.x11_runtime,
                                Some(newc),
                            );
                        }
                    }
                }
                return;
            }
        }
    }

    // 4. Handle Monitor Switch
    if focusfollowsmouse {
        if let Some(new_mon_id) = ctx.core.g.monitors.win_to_mon(
            event_win,
            ctx.x11_runtime.root,
            &*ctx.core.g.clients,
            None,
        ) {
            if new_mon_id != selmon_id {
                ctx.core.g.set_selected_monitor(new_mon_id);
                crate::focus::focus_soft_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, None);
                return;
            }
        }
    }

    // 5. Determine what's actually under the cursor
    let topmost_win_under_cursor =
        crate::mouse::get_cursor_client_win_x11(&ctx.core, &ctx.x11, ctx.x11_runtime);

    // 6. Handle focus switching based on configuration
    crate::focus::hover_focus_target_x11(
        &mut ctx.core,
        &ctx.x11,
        ctx.x11_runtime,
        topmost_win_under_cursor,
        entering_root,
    );
}

pub fn expose(ctx: &mut WmCtxX11<'_>, e: &ExposeEvent) {
    if e.count != 0 {
        return;
    };

    let event_win = WindowId::from(e.window);
    if let Some(monitor_id) =
        ctx.core
            .g
            .monitors
            .win_to_mon(event_win, ctx.x11_runtime.root, &*ctx.core.g.clients, None)
    {
        let is_bar_win = ctx
            .core
            .g
            .monitors
            .get(monitor_id)
            .is_some_and(|m| event_win == m.bar_win);
        if is_bar_win {
            draw_bar(
                &mut ctx.core,
                &ctx.x11,
                ctx.x11_runtime,
                ctx.systray.as_deref(),
                monitor_id,
            );
        }
    };
}

pub fn focus_in(ctx: &mut WmCtxX11<'_>, _e: &FocusInEvent) {
    if let Some(selected_window) = ctx.core.selected_client() {
        crate::client::set_focus_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, selected_window);
    };
}

pub fn mapping_notify(ctx: &mut WmCtxX11<'_>, _e: &MappingNotifyEvent) {
    grab_keys_x11(&ctx.core, &ctx.x11, ctx.x11_runtime);
}

pub fn map_request(ctx: &mut WmCtxX11<'_>, e: &MapRequestEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(_icon) = systray::win_to_systray_icon(&ctx.core, ctx.systray.as_deref(), event_win)
    {
        systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref_mut(),
        );
        return;
    };

    if win_to_client_ctx(&ctx.core, event_win).is_none()
        && !is_override_redirect(&ctx.core, &ctx.x11, event_win)
    {
        let (geo, border_width) = get_win_geometry(&ctx.core, &ctx.x11, event_win);
        let mut tmp = ctx.reborrow();
        manage(&mut tmp, event_win, geo, border_width);
    };
}

/// Handle mouse motion events for bar gesture detection and focus-follows-mouse.
pub fn motion_notify(ctx: &mut WmCtxX11<'_>, e: &MotionNotifyEvent) {
    let event_win = WindowId::from(e.event);
    let root_win = WindowId::from(ctx.x11_runtime.root);
    if event_win != root_win {
        let root_y = e.root_y as i32;
        let selmon = ctx.core.g.selected_monitor();
        let in_bar = selmon.showbar
            && root_y >= selmon.bar_y
            && root_y < selmon.bar_y + ctx.core.g.cfg.bar_height;
        if !in_bar && selmon.gesture != Gesture::None {
            reset_bar_x11(
                &mut ctx.core,
                &ctx.x11,
                ctx.x11_runtime,
                ctx.systray.as_deref(),
            );
        }
        return;
    }

    let selmon_id = ctx.core.g.selected_monitor_id();

    let root_x = e.root_x as i32;
    let root_y = e.root_y as i32;

    // Handle focus-follows-mouse monitor switching
    if ctx.core.g.focusfollowsmouse {
        let rect = Rect {
            x: root_x,
            y: root_y,
            w: 1,
            h: 1,
        };
        if let Some(new_mon) =
            crate::types::find_monitor_by_rect(ctx.core.g.monitors.monitors(), &rect)
                .or(Some(ctx.core.g.selected_monitor_id()))
        {
            if new_mon != selmon_id {
                ctx.core.g.set_selected_monitor(new_mon);
                crate::focus::focus_soft_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, None);
                return;
            }
        }
    };

    // Early-out: cursor is below the bar area.
    let (monitor_y, bar_height, current_gesture) = {
        let mon = ctx.core.g.selected_monitor();
        (mon.monitor_rect.y, ctx.core.g.cfg.bar_height, mon.gesture)
    };

    if root_y >= monitor_y + bar_height {
        if handle_floating_resize_hover(&mut WmCtx::X11(ctx.reborrow()), root_x, root_y, true) {
            return;
        }
        if handle_sidebar_hover(&mut WmCtx::X11(ctx.reborrow()), root_x, root_y) {
            return;
        }
        reset_bar_x11(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref(),
        );
        if ctx.core.g.altcursor == AltCursor::Sidebar {
            reset_cursor_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime);
        }
        return;
    };

    // Cache tag-strip width only when we are actually in the bar hot path.
    ctx.core.g.tags.width = get_tag_width(&ctx.core);

    // Compute the bar position from the cursor's monitor-local x coordinate,
    // then convert to a gesture for hover highlighting.
    let new_gesture = {
        let mon = ctx.core.g.selected_monitor();
        let local_x = root_x - mon.work_rect.x;
        let position = bar_position_at_x(mon, &ctx.core, local_x);
        match position {
            // The status-text and root areas don't produce a hover gesture —
            // reset the bar and bail out so we don't light up anything.
            BarPosition::StatusText | BarPosition::Root => {
                reset_bar_x11(
                    &mut ctx.core,
                    &ctx.x11,
                    ctx.x11_runtime,
                    ctx.systray.as_deref(),
                );
                return;
            }
            other => bar_position_to_gesture(other),
        }
    };

    if new_gesture != current_gesture {
        ctx.core.g.selected_monitor_mut().gesture = new_gesture;
        draw_bar(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref(),
            selmon_id,
        );
    };
}

pub fn property_notify(ctx: &mut WmCtxX11<'_>, e: &PropertyNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(_icon) = systray::win_to_systray_icon(&ctx.core, ctx.systray.as_deref(), event_win)
    {
        systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref_mut(),
        );
        return;
    };

    if event_win == WindowId::from(ctx.x11_runtime.root) && e.atom == AtomEnum::WM_NAME.into() {
        crate::bar::x11::update_status(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref_mut(),
        );
        return;
    };

    if let Some(win) = win_to_client_ctx(&ctx.core, event_win) {
        match e.atom {
            x if x == AtomEnum::WM_NORMAL_HINTS.into() => {
                if let Some(c) = ctx.core.g.clients.get_mut(&win) {
                    c.hintsvalid = 0;
                }
            }
            x if x == AtomEnum::WM_HINTS.into() => {
                update_wm_hints(&mut ctx.core, &ctx.x11, win);
                draw_bars_x11(
                    &mut ctx.core,
                    &ctx.x11,
                    ctx.x11_runtime,
                    ctx.systray.as_deref(),
                );
            }
            _ => {}
        }

        let net_wm_name = ctx.x11_runtime.netatom.wm_name;
        if e.atom == AtomEnum::WM_NAME.into() || e.atom == net_wm_name {
            update_title_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, win);
        }
    };
}

pub fn resize_request(ctx: &mut WmCtxX11<'_>, e: &ResizeRequestEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(_icon) = systray::win_to_systray_icon(&ctx.core, ctx.systray.as_deref(), event_win)
    {
        systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref_mut(),
        );
    };
}

pub fn unmap_notify(ctx: &mut WmCtxX11<'_>, e: &UnmapNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(win) = win_to_client_ctx(&ctx.core, event_win) {
        if e.response_type & 0x80 != 0 {
            set_client_state(
                &ctx.core,
                &ctx.x11,
                ctx.x11_runtime,
                win,
                WM_STATE_WITHDRAWN,
            );
        } else {
            let mut tmp = ctx.reborrow();
            unmanage(&mut tmp, win, false);
        }
    } else if let Some(_icon) =
        systray::win_to_systray_icon(&ctx.core, ctx.systray.as_deref(), event_win)
    {
        // Systray icons sometimes unmap without destroying; re-map them.
        systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref_mut(),
        );
    };
}

pub fn leave_notify(ctx: &mut WmCtxX11<'_>, _e: &LeaveNotifyEvent) {
    reset_bar_x11(
        &mut ctx.core,
        &ctx.x11,
        ctx.x11_runtime,
        ctx.systray.as_deref(),
    );
}

fn handle_systray_dock_request(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent) {
    let data = e.data.as_data32();
    let icon_win = WindowId::from(data[2]);
    if icon_win == WindowId::default() {
        return;
    };

    let selmon_id = ctx.core.g.selected_monitor_id();
    let systray_win_opt = ctx.systray.as_ref().map(|s| s.win);
    let statusescheme_bg_pixel = ctx
        .core
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
        let conn = ctx.x11.conn;
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
        ctx.core.g.clients.insert(icon_win, client);
        if let Some(ref mut systray) = ctx.systray {
            systray.icons.insert(0, icon_win);
        }
    };

    crate::backend::x11::update_size_hints_x11(&mut ctx.core, &ctx.x11, icon_win);
    systray::update_systray_icon_geom(&mut ctx.core, &ctx.x11, icon_win, geo.w, geo.h);

    let conn = ctx.x11.conn;
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

    let xembed_atom = ctx.x11_runtime.xatom.xembed;
    let structure_notify_mask = EventMask::STRUCTURE_NOTIFY.bits();

    crate::client::send_event_x11(
        &mut ctx.core,
        &ctx.x11,
        ctx.x11_runtime,
        icon_win,
        xembed_atom,
        structure_notify_mask,
        CURRENT_TIME as i64,
        XEMBED_EMBEDDED_NOTIFY as i64,
        0,
        u32::from(systray_win) as i64,
        XEMBED_EMBEDDED_VERSION as i64,
    );
    crate::client::send_event_x11(
        &mut ctx.core,
        &ctx.x11,
        ctx.x11_runtime,
        icon_win,
        xembed_atom,
        structure_notify_mask,
        CURRENT_TIME as i64,
        XEMBED_FOCUS_IN as i64,
        0,
        u32::from(systray_win) as i64,
        XEMBED_EMBEDDED_VERSION as i64,
    );
    crate::client::send_event_x11(
        &mut ctx.core,
        &ctx.x11,
        ctx.x11_runtime,
        icon_win,
        xembed_atom,
        structure_notify_mask,
        CURRENT_TIME as i64,
        XEMBED_WINDOW_ACTIVATE as i64,
        0,
        u32::from(systray_win) as i64,
        XEMBED_EMBEDDED_VERSION as i64,
    );
    crate::client::send_event_x11(
        &mut ctx.core,
        &ctx.x11,
        ctx.x11_runtime,
        icon_win,
        xembed_atom,
        structure_notify_mask,
        CURRENT_TIME as i64,
        XEMBED_MODALITY_ON as i64,
        0,
        u32::from(systray_win) as i64,
        XEMBED_EMBEDDED_VERSION as i64,
    );

    if let Some(mon) = ctx.core.g.monitor(selmon_id).cloned() {
        crate::bar::resize_bar_win(
            &ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref(),
            &mon,
        );
    };

    systray::update_systray(
        &mut ctx.core,
        &ctx.x11,
        ctx.x11_runtime,
        ctx.systray.as_deref_mut(),
    );
    set_client_state(&ctx.core, &ctx.x11, ctx.x11_runtime, icon_win, 1);
}

fn handle_net_wm_state(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent, win: WindowId) {
    let data = e.data.as_data32();
    let fullscreen_action = data[0];

    if fullscreen_action == 1 {
        set_fullscreen_x11(ctx, win, true);
    } else if fullscreen_action == 0 {
        set_fullscreen_x11(ctx, win, false);
    };
}

fn handle_active_window(ctx: &mut WmCtxX11<'_>, win: WindowId) {
    let is_hidden = ctx.core.g.clients.is_hidden(win);
    if is_hidden {
        crate::client::show(&mut WmCtx::X11(ctx.reborrow()), win);
    };

    if let Some(c) = ctx.core.g.clients.get(&win) {
        if let Some(monitor_id) = c.monitor_id {
            crate::focus::focus_soft_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, Some(win));
            restack(&mut WmCtx::X11(ctx.reborrow()), monitor_id);
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
            wm.backend.flush();

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
    let ctx = wm.ctx();
    let crate::contexts::WmCtx::X11(mut ctx) = ctx else {
        return;
    };

    match event {
        x11rb::protocol::Event::ButtonPress(e) => button_press_x11(&mut ctx, &e),
        x11rb::protocol::Event::ClientMessage(e) => client_message(&mut ctx, &e),
        x11rb::protocol::Event::ConfigureNotify(e) => configure_notify(&mut ctx, &e),
        x11rb::protocol::Event::ConfigureRequest(e) => configure_request(&mut ctx, &e),
        x11rb::protocol::Event::CreateNotify(e) => create_notify(&e),
        x11rb::protocol::Event::DestroyNotify(e) => destroy_notify(&mut ctx, &e),
        x11rb::protocol::Event::EnterNotify(e) => enter_notify(&mut ctx, &e),
        x11rb::protocol::Event::Expose(e) => expose(&mut ctx, &e),
        x11rb::protocol::Event::FocusIn(e) => focus_in(&mut ctx, &e),
        x11rb::protocol::Event::KeyPress(e) => key_press_x11(&mut ctx, &e),
        x11rb::protocol::Event::KeyRelease(e) => key_release_x11(&mut ctx, &e),
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
fn get_win_geometry(_core: &CoreCtx, x11: &X11BackendRef, win: WindowId) -> (Rect, u32) {
    let conn = x11.conn;
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
fn is_override_redirect(_core: &CoreCtx, x11: &X11BackendRef, win: WindowId) -> bool {
    let conn = x11.conn;
    let x11_win: Window = win.into();
    conn.get_window_attributes(x11_win)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|wa| wa.override_redirect)
        .unwrap_or(false)
}

/// Partition `children` into `(managed, transients)`.
fn classify_windows(
    core: &CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &crate::globals::X11RuntimeConfig,
    children: Vec<Window>,
) -> (Vec<WindowId>, Vec<WindowId>) {
    let mut managed = Vec::new();
    let mut transients = Vec::new();

    let conn = x11.conn;

    for win in children {
        let win_id = WindowId::from(win);
        if is_override_redirect(core, x11, win_id) {
            continue;
        }

        // Skip windows that are neither visible nor iconic.
        let is_viewable = conn
            .get_window_attributes(win)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|wa| wa.map_state == MapState::VIEWABLE)
            .unwrap_or(false);
        let is_iconic = is_window_iconic(core, x11, x11_runtime, win_id);

        if !is_viewable && !is_iconic {
            continue;
        }

        // Skip already-managed windows.
        if win_to_client_ctx(core, win_id).is_some() {
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

fn is_window_iconic(
    _core: &CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &crate::globals::X11RuntimeConfig,
    win: WindowId,
) -> bool {
    let conn = x11.conn;
    let x11_win: Window = win.into();

    let state_atom = x11_runtime.wmatom.state;
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
    let ctx = wm.ctx();
    let crate::contexts::WmCtx::X11(mut ctx) = ctx else {
        return;
    };
    let conn = ctx.x11.conn;
    let root = ctx.x11_runtime.root;

    let children = {
        let Ok(tree_cookie) = conn.query_tree(root) else {
            return;
        };
        let Ok(tree_reply) = tree_cookie.reply() else {
            return;
        };
        tree_reply.children
    };

    let (managed, transients) = classify_windows(&ctx.core, &ctx.x11, ctx.x11_runtime, children);

    for win in managed.into_iter().chain(transients) {
        let (geo, border_width) = get_win_geometry(&ctx.core, &ctx.x11, win);
        let mut tmp = ctx.reborrow();
        manage(&mut tmp, win, geo, border_width);
    }
}

pub fn check_other_wm(conn: &x11rb::rust_connection::RustConnection, root: Window) {
    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    let result =
        conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(mask));

    if result.is_err() || conn.flush().is_err() {
        crate::util::die("instantwm: another window manager is already running");
    }
}

pub fn setup(_wm: &mut Wm) {}

pub fn setup_root(wm: &mut Wm) {
    let Some(x11) = wm.backend.x11() else {
        return;
    };
    let conn = &x11.conn;
    let root = wm.x11_runtime.root;
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

    if let WmCtx::X11(mut x11_ctx) = ctx {
        crate::mouse::set_cursor_default_x11(&mut x11_ctx);
    }
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
