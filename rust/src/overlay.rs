use crate::animation::animate_client;
use crate::client::save_border_width;
use crate::client::{attach, attach_stack, detach, detach_stack, resize};
use crate::constants::animation::OVERLAY_ANIMATION_FRAMES;
use crate::constants::overlay::*;
use crate::contexts::WmCtx;
use crate::focus::focus;
use crate::layouts::arrange;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

//TODO: maybe overlay should be a struct with the overlay relevant state kept
//there

/// Information needed to position an overlay window.
#[derive(Debug, Clone, Copy)]
struct OverlayPositionInfo {
    mode: OverlayMode,
    /// Monitor rectangle (position and total size).
    monitor_rect: Rect,
    /// Work area width (excluding bars/padding).
    work_width: i32,
    /// Y offset from top (accounting for bar height).
    yoffset: i32,
    /// Client size.
    client_size: Rect,
}

/// Get the overlay window for the selected monitor, if it exists.
fn get_overlay_win(ctx: &WmCtx) -> Option<Window> {
    ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.overlay)
}

/// Check if the overlay window exists in the clients map.
pub fn overlay_exists(ctx: &WmCtx) -> bool {
    get_overlay_win(ctx).is_some_and(|win| ctx.g.clients.contains_key(&win))
}

/// Raise a window to the top of the stack.
fn raise_window(ctx: &WmCtx, win: Window) {
    {
        let conn = ctx.x11.conn;
        let _ = conn.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        let _ = conn.flush();
    }
}

/// Calculate the y offset based on showbar and fullscreen clients.
fn calculate_yoffset(ctx: &WmCtx, mon: &Monitor, current_tag: u32) -> i32 {
    let bh = ctx.g.cfg.bh;
    let base_offset = if mon.showbar { bh } else { 0 };

    // Check if any visible client is fullscreen
    for (_win, c) in mon.iter_clients(&ctx.g.clients) {
        if (c.tags & (1 << (current_tag - 1))) != 0 && c.is_fullscreen && !c.isfakefullscreen {
            return 0;
        }
    }

    base_offset
}

/// Get the initial position rect for the overlay (off-screen, for animation start).
fn get_initial_overlay_rect(info: &OverlayPositionInfo) -> Rect {
    let OverlayPositionInfo {
        mode,
        monitor_rect,
        work_width,
        yoffset,
        client_size,
    } = *info;

    match mode {
        OverlayMode::Top => Rect {
            x: monitor_rect.x + OVERLAY_MARGIN_X,
            y: monitor_rect.y + yoffset - client_size.h,
            w: work_width - OVERLAY_INSET_X,
            h: client_size.h,
        },
        OverlayMode::Right => Rect {
            x: monitor_rect.x + monitor_rect.w - OVERLAY_MARGIN_X,
            y: monitor_rect.y + OVERLAY_MARGIN_Y,
            w: client_size.w,
            h: monitor_rect.h - OVERLAY_INSET_Y,
        },
        OverlayMode::Bottom => Rect {
            x: monitor_rect.x + OVERLAY_MARGIN_X,
            y: monitor_rect.y + monitor_rect.h,
            w: work_width - OVERLAY_INSET_X,
            h: client_size.h,
        },
        OverlayMode::Left => Rect {
            x: monitor_rect.x - client_size.w + OVERLAY_MARGIN_X,
            y: monitor_rect.y + OVERLAY_MARGIN_Y,
            w: client_size.w,
            h: monitor_rect.h - OVERLAY_INSET_Y,
        },
    }
}

/// Get the target position rect for the overlay (visible position after animation).
fn get_target_overlay_rect(info: &OverlayPositionInfo) -> Rect {
    let OverlayPositionInfo {
        mode,
        monitor_rect,
        work_width,
        yoffset,
        client_size,
    } = *info;

    match mode {
        OverlayMode::Top => Rect {
            x: monitor_rect.x + OVERLAY_MARGIN_X,
            y: monitor_rect.y + yoffset,
            w: work_width - OVERLAY_INSET_X,
            h: client_size.h,
        },
        OverlayMode::Right => Rect {
            x: monitor_rect.x + monitor_rect.w - client_size.w,
            y: monitor_rect.y + OVERLAY_MARGIN_Y,
            w: client_size.w,
            h: monitor_rect.h - OVERLAY_INSET_Y,
        },
        OverlayMode::Bottom => Rect {
            x: monitor_rect.x + OVERLAY_MARGIN_X,
            y: monitor_rect.y + monitor_rect.h - client_size.h,
            w: work_width - OVERLAY_INSET_X,
            h: client_size.h,
        },
        OverlayMode::Left => Rect {
            x: monitor_rect.x,
            y: monitor_rect.y + OVERLAY_MARGIN_Y,
            w: client_size.w,
            h: monitor_rect.h - OVERLAY_INSET_Y,
        },
    }
}

/// Information needed for hide animation.
#[derive(Debug, Clone, Copy)]
struct HideAnimationInfo {
    mode: OverlayMode,
    /// Monitor rectangle (position and total size).
    monitor_rect: Rect,
    /// Client position x (for Top/Bottom animation).
    client_x: i32,
    /// Client size.
    client_size: Rect,
}

/// Get the target rect for hiding the overlay (off-screen position).
fn get_hide_animation_rect(info: &HideAnimationInfo) -> Rect {
    let HideAnimationInfo {
        mode,
        monitor_rect,
        client_x,
        client_size,
    } = *info;

    match mode {
        OverlayMode::Top => Rect {
            x: client_x,
            y: -client_size.h,
            w: 0,
            h: 0,
        },
        OverlayMode::Right => Rect {
            x: monitor_rect.x + monitor_rect.w,
            y: monitor_rect.y + OVERLAY_MARGIN_Y,
            w: 0,
            h: 0,
        },
        OverlayMode::Bottom => Rect {
            x: client_x,
            y: monitor_rect.y + monitor_rect.h,
            w: 0,
            h: 0,
        },
        OverlayMode::Left => Rect {
            x: monitor_rect.x - client_size.w,
            y: OVERLAY_MARGIN_Y,
            w: 0,
            h: 0,
        },
    }
}

/// Create overlay with dependency injection.
pub fn create_overlay(ctx: &mut WmCtx, sel_win: Window) {
    let (sel_overlay, sel_fullscreen) = {
        let g = &*ctx.g;
        let mon = match g.monitors.get(g.selmon) {
            Some(m) => m,
            None => return,
        };
        let sel_overlay = mon.overlay;
        let sel_fullscreen = g
            .clients
            .get(&sel_win)
            .map(|c| c.is_fullscreen && !c.isfakefullscreen)
            .unwrap_or(false);
        (sel_overlay, sel_fullscreen)
    };

    if sel_fullscreen {
        crate::floating::temp_fullscreen(ctx);
    }

    if Some(sel_win) == sel_overlay {
        reset_overlay(ctx);
        for mon in ctx.g.monitors.iter_mut() {
            mon.overlay = None;
        }
        return;
    }

    let temp_client = sel_win;

    reset_overlay(ctx);

    for mon in ctx.g.monitors.iter_mut() {
        mon.overlay = Some(temp_client);
        mon.overlaystatus = 0;
    }

    save_border_width(temp_client);

    if let Some(client) = ctx.g.clients.get_mut(&temp_client) {
        client.border_width = 0;
        client.islocked = true;

        if !client.isfloating {
            client.isfloating = true;
        }
    }

    let (overlay_mode, mon_ww, mon_wh) = match ctx.g.monitors.get(ctx.g.selmon) {
        Some(mon) => (mon.overlaymode, mon.work_rect.w, mon.work_rect.h),
        None => (OverlayMode::default(), 0, 0),
    };

    if let Some(client) = ctx.g.clients.get_mut(&temp_client) {
        if overlay_mode.is_vertical() {
            client.geo.h = mon_wh / 3;
        } else {
            client.geo.w = mon_ww / 3;
        }
    }

    {
        let conn = ctx.x11.conn;
        let _ = conn.configure_window(
            temp_client,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
        let _ = conn.flush();
    }

    show_overlay(ctx);
}

pub fn reset_overlay(ctx: &mut WmCtx) {
    if !overlay_exists(ctx) {
        return;
    }

    let overlay_win = match ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.overlay) {
        Some(w) => w,
        None => return,
    };

    let selmon = ctx.g.selmon;

    if let Some(client) = ctx.g.clients.get_mut(&overlay_win) {
        client.border_width = client.old_border_width;
        client.issticky = false;
        client.islocked = false;
        client.isfloating = true;
    }

    arrange(ctx, Some(selmon));

    focus(ctx, Some(overlay_win));
}

/// Prepare the overlay window for display (detach, update state, reattach).
fn prepare_overlay_window(ctx: &mut WmCtx, overlay_win: Window, selmon_id: MonitorId) {
    detach(overlay_win);
    detach_stack(overlay_win);

    if let Some(client) = ctx.g.clients.get_mut(&overlay_win) {
        client.mon_id = Some(selmon_id);
        client.isfloating = true;
    }

    attach(overlay_win);
    attach_stack(overlay_win);
}

/// Update overlay client properties for showing.
fn update_overlay_client_for_show(ctx: &mut WmCtx, overlay_win: Window, tags: u32) {
    if let Some(client) = ctx.g.clients.get_mut(&overlay_win) {
        if !client.isfloating {
            client.isfloating = true;
        }
        client.border_width = 0;
        client.tags = tags;
    }
}

pub fn show_overlay(ctx: &mut WmCtx) {
    if !overlay_exists(ctx) || ctx.g.monitors.is_empty() {
        return;
    }

    let selmon_id = ctx.g.selmon;
    let mon = match ctx.g.monitors.get(selmon_id) {
        Some(m) => m,
        None => return,
    };

    if mon.overlaystatus != 0 {
        return;
    }

    let overlay_win = match mon.overlay {
        Some(w) => w,
        None => return,
    };

    let current_tag = mon.current_tag as u32;
    let yoffset = calculate_yoffset(ctx, mon, current_tag);

    // Mark overlay as shown on all monitors
    for mon in &mut ctx.g.monitors {
        mon.overlaystatus = 1;
    }

    prepare_overlay_window(ctx, overlay_win, selmon_id);

    // Gather all needed data in one place
    let (overlay_mode, mon_rect, mon_ww, is_locked, client_w, client_h) = {
        let mon = ctx.g.monitors.get(selmon_id).unwrap();
        let client = match ctx.g.clients.get(&overlay_win) {
            Some(c) => c,
            None => return,
        };
        (
            mon.overlaymode,
            mon.monitor_rect,
            mon.work_rect.w,
            client.islocked,
            client.geo.w,
            client.geo.h,
        )
    };

    let pos_info = OverlayPositionInfo {
        mode: overlay_mode,
        monitor_rect: mon_rect,
        work_width: mon_ww,
        yoffset,
        client_size: Rect {
            x: 0,
            y: 0,
            w: client_w,
            h: client_h,
        },
    };

    if is_locked {
        let initial_rect = get_initial_overlay_rect(&pos_info);
        resize(ctx, overlay_win, &initial_rect, true);
    }

    let tags = ctx.g.monitors.get(selmon_id).unwrap().tagset
        [ctx.g.monitors.get(selmon_id).unwrap().seltags as usize];
    update_overlay_client_for_show(ctx, overlay_win, tags);

    if is_locked {
        raise_window(ctx, overlay_win);

        let target_rect = get_target_overlay_rect(&pos_info);
        animate_client(
            ctx,
            overlay_win,
            &target_rect.with_size(0, 0), // animate_client uses size from client, only x/y matter
            OVERLAY_ANIMATION_FRAMES,
            0,
        );

        if let Some(client) = ctx.g.clients.get_mut(&overlay_win) {
            client.issticky = true;
        }
    }

    focus(ctx, Some(overlay_win));
    raise_window(ctx, overlay_win);
}

/// Check if overlay is fullscreen on the given monitor.
fn is_overlay_fullscreen(_ctx: &WmCtx, overlay_win: Window, mon: &Monitor) -> bool {
    mon.fullscreen == Some(overlay_win)
}

/// Clear overlay tags and sticky state.
fn clear_overlay_state(ctx: &mut WmCtx, overlay_win: Window) {
    if let Some(client) = ctx.g.clients.get_mut(&overlay_win) {
        client.issticky = false;
        client.tags = 0;
    }
}

/// Reset overlay status on all monitors.
fn reset_all_overlay_status(monitors: &mut [Monitor]) {
    for mon in monitors {
        mon.overlaystatus = 0;
    }
}

pub fn hide_overlay(ctx: &mut WmCtx) {
    if !overlay_exists(ctx) || ctx.g.monitors.is_empty() {
        return;
    }

    let selmon_id = ctx.g.selmon;
    let mon = match ctx.g.monitors.get(selmon_id) {
        Some(m) => m,
        None => return,
    };

    if mon.overlaystatus == 0 {
        return;
    }

    let overlay_win = match mon.overlay {
        Some(w) => w,
        None => return,
    };

    // Gather all needed data
    let (is_locked, is_fullscreen, hide_info) = {
        let client = match ctx.g.clients.get(&overlay_win) {
            Some(c) => c,
            None => return,
        };
        let mon = ctx.g.monitors.get(selmon_id).unwrap();

        let hide_info = HideAnimationInfo {
            mode: mon.overlaymode,
            monitor_rect: mon.monitor_rect,
            client_x: client.geo.x,
            client_size: Rect {
                x: 0,
                y: 0,
                w: client.geo.w,
                h: client.geo.h,
            },
        };

        (
            client.islocked,
            is_overlay_fullscreen(ctx, overlay_win, mon),
            hide_info,
        )
    };

    if is_fullscreen {
        crate::floating::temp_fullscreen(ctx);
    }

    clear_overlay_state(ctx, overlay_win);

    if is_locked {
        let hide_rect = get_hide_animation_rect(&hide_info);
        animate_client(ctx, overlay_win, &hide_rect, OVERLAY_ANIMATION_FRAMES, 0);
    }

    reset_all_overlay_status(&mut ctx.g.monitors);

    focus(ctx, None);
    arrange(ctx, Some(selmon_id));
}

pub fn set_overlay(ctx: &mut WmCtx) {
    if !overlay_exists(ctx) {
        return;
    }

    let (overlaystatus, overlay_visible, _mon_tags) = {
        if let Some(mon) = ctx.g.monitors.get(ctx.g.selmon) {
            let overlay_win = match mon.overlay {
                Some(w) => w,
                None => return,
            };

            let visible = if let Some(c) = ctx.g.clients.get(&overlay_win) {
                let selected = mon.selected_tags();
                c.is_visible_on_tags(selected)
            } else {
                false
            };

            (mon.overlaystatus, visible, mon.tagset[mon.seltags as usize])
        } else {
            return;
        }
    };

    if overlaystatus == 0 {
        show_overlay(ctx);
    } else if overlay_visible {
        hide_overlay(ctx);
    } else {
        show_overlay(ctx);
    }
}

pub fn set_overlay_mode(ctx: &mut WmCtx, mode: OverlayMode) {
    for mon in &mut ctx.g.monitors {
        mon.overlaymode = mode;
    }

    let (has_overlay, mon_wh, mon_ww, overlaystatus) = {
        let mon = ctx.g.monitors.get(ctx.g.selmon);
        (
            mon.and_then(|m| m.overlay).is_some(),
            mon.map(|m| m.work_rect.h).unwrap_or(0),
            mon.map(|m| m.work_rect.w).unwrap_or(0),
            mon.map(|m| m.overlaystatus).unwrap_or(0),
        )
    };

    if !has_overlay {
        return;
    }

    if let Some(mon) = ctx.g.monitors.get(ctx.g.selmon) {
        if let Some(overlay_win) = mon.overlay {
            if let Some(client) = ctx.g.clients.get_mut(&overlay_win) {
                if mode.is_vertical() {
                    client.geo.h = mon_wh / 3;
                } else {
                    client.geo.w = mon_ww / 3;
                }
            }
        }
    }

    if overlaystatus != 0 {
        hide_overlay(ctx);
        show_overlay(ctx);
    }
}
