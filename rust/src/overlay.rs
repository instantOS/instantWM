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

pub fn overlay_exists(ctx: &WmCtx) -> bool {
    let overlay_win = match ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.overlay) {
        Some(w) => w,
        None => return false,
    };

    ctx.g.clients.contains_key(&overlay_win)
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

    if let Some(ref conn) = ctx.x11.conn {
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

pub fn show_overlay(ctx: &mut WmCtx) {
    if !overlay_exists(ctx) {
        return;
    }

    if ctx.g.monitors.is_empty() {
        return;
    }

    let selmon_id = ctx.g.selmon;

    let (overlaystatus, showbar, overlay_win, current_tag, mut current) = {
        let mon = match ctx.g.monitors.get(selmon_id) {
            Some(m) => m,
            None => return,
        };
        let overlay_win = match mon.overlay {
            Some(w) => w,
            None => return,
        };
        (
            mon.overlaystatus,
            mon.showbar,
            overlay_win,
            mon.current_tag as u32,
            mon.clients,
        )
    };

    if overlaystatus != 0 {
        return;
    }

    let bh = ctx.g.cfg.bh;
    let mut yoffset = if showbar { bh } else { 0 };

    while let Some(c_win) = current {
        if let Some(c) = ctx.g.clients.get(&c_win) {
            if (c.tags & (1 << (current_tag - 1))) != 0 && c.is_fullscreen && !c.isfakefullscreen {
                yoffset = 0;
                break;
            }
            current = c.next;
        } else {
            break;
        }
    }

    for mon in &mut ctx.g.monitors {
        mon.overlaystatus = 1;
    }

    detach(overlay_win);
    detach_stack(overlay_win);

    if let Some(client) = ctx.g.clients.get_mut(&overlay_win) {
        client.mon_id = Some(selmon_id);
        client.isfloating = true;
    }

    attach(overlay_win);
    attach_stack(overlay_win);

    let (overlay_mode, mon_mx, mon_my, mon_mw, mon_mh, mon_ww) = {
        if let Some(mon) = ctx.g.monitors.get(selmon_id) {
            (
                mon.overlaymode,
                mon.monitor_rect.x,
                mon.monitor_rect.y,
                mon.monitor_rect.w,
                mon.monitor_rect.h,
                mon.work_rect.w,
            )
        } else {
            return;
        }
    };

    let (client_w, client_h, is_locked) = {
        if let Some(c) = ctx.g.clients.get(&overlay_win) {
            (c.geo.w, c.geo.h, c.islocked)
        } else {
            return;
        }
    };

    if is_locked {
        match overlay_mode {
            OverlayMode::Top => {
                resize(
                    ctx,
                    overlay_win,
                    &Rect {
                        x: mon_mx + OVERLAY_MARGIN_X,
                        y: mon_my + yoffset - client_h,
                        w: mon_ww - OVERLAY_INSET_X,
                        h: client_h,
                    },
                    true,
                );
            }
            OverlayMode::Right => {
                resize(
                    ctx,
                    overlay_win,
                    &Rect {
                        x: mon_mx + mon_mw - OVERLAY_MARGIN_X,
                        y: mon_my + OVERLAY_MARGIN_Y,
                        w: client_w,
                        h: mon_mh - OVERLAY_INSET_Y,
                    },
                    true,
                );
            }
            OverlayMode::Bottom => {
                resize(
                    ctx,
                    overlay_win,
                    &Rect {
                        x: mon_mx + OVERLAY_MARGIN_X,
                        y: mon_my + mon_mh,
                        w: mon_ww - OVERLAY_INSET_X,
                        h: client_h,
                    },
                    true,
                );
            }
            OverlayMode::Left => {
                resize(
                    ctx,
                    overlay_win,
                    &Rect {
                        x: mon_mx - client_w + OVERLAY_MARGIN_X,
                        y: mon_my + OVERLAY_MARGIN_Y,
                        w: client_w,
                        h: mon_mh - OVERLAY_INSET_Y,
                    },
                    true,
                );
            }
        }
    }

    if let Some(mon) = ctx.g.monitors.get(selmon_id) {
        if ctx.g.clients.contains_key(&overlay_win) {
            let tags = mon.tagset[mon.seltags as usize];
            if let Some(c) = ctx.g.clients.get_mut(&overlay_win) {
                c.tags = tags;
            }
        }
    }

    if let Some(client) = ctx.g.clients.get_mut(&overlay_win) {
        if !client.isfloating {
            client.isfloating = true;
        }
        client.border_width = 0;
    }

    if is_locked {
        if let Some(ref conn) = ctx.x11.conn {
            let _ = conn.configure_window(
                overlay_win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
            let _ = conn.flush();
        }

        let (target_x, target_y) = {
            if let Some(mon) = ctx.g.monitors.get(selmon_id) {
                match overlay_mode {
                    OverlayMode::Top => (overlay_win as i32, mon.monitor_rect.y + yoffset),
                    OverlayMode::Right => (
                        mon.monitor_rect.x + mon.monitor_rect.w - client_w,
                        mon.monitor_rect.y + OVERLAY_MARGIN_Y,
                    ),
                    OverlayMode::Bottom => (
                        mon.monitor_rect.x + OVERLAY_MARGIN_X,
                        mon.monitor_rect.y + mon.monitor_rect.h - client_h,
                    ),
                    OverlayMode::Left => {
                        (mon.monitor_rect.x, mon.monitor_rect.y + OVERLAY_MARGIN_Y)
                    }
                }
            } else {
                (0, 0)
            }
        };

        animate_client(
            ctx,
            overlay_win,
            &Rect {
                x: target_x,
                y: target_y,
                w: 0,
                h: 0,
            },
            OVERLAY_ANIMATION_FRAMES,
            0,
        );

        if let Some(client) = ctx.g.clients.get_mut(&overlay_win) {
            client.issticky = true;
        }
    }

    focus(ctx, Some(overlay_win));

    if let Some(ref conn) = ctx.x11.conn {
        let _ = conn.configure_window(
            overlay_win,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
        let _ = conn.flush();
    }
}

pub fn hide_overlay(ctx: &mut WmCtx) {
    if !overlay_exists(ctx) {
        return;
    }

    if ctx.g.monitors.is_empty() {
        return;
    }

    let selmon_id = ctx.g.selmon;

    let (overlaystatus, overlay_win) = {
        let mon = match ctx.g.monitors.get(selmon_id) {
            Some(m) => m,
            None => return,
        };
        let overlay_win = match mon.overlay {
            Some(w) => w,
            None => return,
        };
        (mon.overlaystatus, overlay_win)
    };

    if overlaystatus == 0 {
        return;
    }

    let (
        is_locked,
        overlay_mode,
        mon_mx,
        mon_my,
        mon_mw,
        mon_mh,
        client_x,
        client_h,
        client_w,
        is_fullscreen,
    ) = {
        if let Some(c) = ctx.g.clients.get(&overlay_win) {
            let mon = ctx.g.monitors.get(selmon_id).unwrap();
            let is_fullscreen = Some(overlay_win) == mon.fullscreen;
            let is_locked = c.islocked;
            (
                is_locked,
                mon.overlaymode,
                mon.monitor_rect.x,
                mon.monitor_rect.y,
                mon.monitor_rect.w,
                mon.monitor_rect.h,
                c.geo.x,
                c.geo.h,
                c.geo.w,
                is_fullscreen,
            )
        } else {
            return;
        }
    };

    if is_fullscreen {
        crate::floating::temp_fullscreen(ctx);
    }

    if let Some(client) = ctx.g.clients.get_mut(&overlay_win) {
        client.issticky = false;
    }

    if is_locked {
        match overlay_mode {
            OverlayMode::Top => {
                animate_client(
                    ctx,
                    overlay_win,
                    &Rect {
                        x: client_x,
                        y: 0 - client_h,
                        w: 0,
                        h: 0,
                    },
                    OVERLAY_ANIMATION_FRAMES,
                    0,
                );
            }
            OverlayMode::Right => {
                animate_client(
                    ctx,
                    overlay_win,
                    &Rect {
                        x: mon_mx + mon_mw,
                        y: mon_mx + mon_mw,
                        w: 0,
                        h: 0,
                    },
                    OVERLAY_ANIMATION_FRAMES,
                    0,
                );
            }
            OverlayMode::Bottom => {
                animate_client(
                    ctx,
                    overlay_win,
                    &Rect {
                        x: client_x,
                        y: mon_mh + mon_my,
                        w: 0,
                        h: 0,
                    },
                    OVERLAY_ANIMATION_FRAMES,
                    0,
                );
            }
            OverlayMode::Left => {
                animate_client(
                    ctx,
                    overlay_win,
                    &Rect {
                        x: mon_mx - client_w,
                        y: OVERLAY_MARGIN_Y,
                        w: 0,
                        h: 0,
                    },
                    OVERLAY_ANIMATION_FRAMES,
                    0,
                );
            }
        }
    }

    for mon in &mut ctx.g.monitors {
        mon.overlaystatus = 0;
    }

    if let Some(client) = ctx.g.clients.get_mut(&overlay_win) {
        client.tags = 0;
    }

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
                c.is_visible() || c.issticky
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

pub fn is_overlay_window(ctx: &WmCtx, win: Window) -> bool {
    for mon in &ctx.g.monitors {
        if mon.overlay == Some(win) {
            return true;
        }
    }
    false
}

pub fn reset_overlay_size(ctx: &mut WmCtx) {
    let (
        has_overlay,
        overlay_mode,
        mon_mx,
        mon_my,
        mon_mw,
        mon_mh,
        mon_ww,
        mon_wh,
        mon_showbar,
        bh,
    ) = {
        if let Some(mon) = ctx.g.monitors.get(ctx.g.selmon) {
            let overlay_win = mon.overlay;
            (
                overlay_win.is_some(),
                mon.overlaymode,
                mon.monitor_rect.x,
                mon.monitor_rect.y,
                mon.monitor_rect.w,
                mon.monitor_rect.h,
                mon.work_rect.w,
                mon.work_rect.h,
                mon.showbar,
                ctx.g.cfg.bh,
            )
        } else {
            return;
        }
    };

    if !has_overlay {
        return;
    }

    let Some(win) = ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.overlay) else {
        return;
    };

    if let Some(client) = ctx.g.clients.get_mut(&win) {
        client.isfloating = true;
    }

    let yoffset = if mon_showbar { bh } else { 0 };

    match overlay_mode {
        OverlayMode::Top => {
            resize(
                ctx,
                win,
                &Rect {
                    x: mon_mx + OVERLAY_MARGIN_X,
                    y: mon_my + yoffset,
                    w: mon_ww - OVERLAY_INSET_X,
                    h: mon_wh / 3,
                },
                true,
            );
        }
        OverlayMode::Right => {
            let client_w = {
                ctx.g
                    .clients
                    .get(&win)
                    .map(|c| c.geo.w)
                    .unwrap_or(mon_mw / 3)
            };
            resize(
                ctx,
                win,
                &Rect {
                    x: mon_mx + mon_mw - client_w,
                    y: mon_my + OVERLAY_MARGIN_Y,
                    w: mon_mw / 3,
                    h: mon_mh - OVERLAY_INSET_Y,
                },
                true,
            );
        }
        OverlayMode::Bottom => {
            let client_h = {
                ctx.g
                    .clients
                    .get(&win)
                    .map(|c| c.geo.h)
                    .unwrap_or(mon_wh / 3)
            };
            resize(
                ctx,
                win,
                &Rect {
                    x: mon_mx + OVERLAY_MARGIN_X,
                    y: mon_my + mon_mh - client_h,
                    w: mon_ww - OVERLAY_INSET_X,
                    h: mon_wh / 3,
                },
                true,
            );
        }
        OverlayMode::Left => {
            resize(
                ctx,
                win,
                &Rect {
                    x: mon_mx,
                    y: mon_my + OVERLAY_MARGIN_Y,
                    w: mon_mw / 3,
                    h: mon_mh - OVERLAY_INSET_Y,
                },
                true,
            );
        }
    }
}
