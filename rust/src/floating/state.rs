//! Floating state transitions and geometry persistence.

use crate::animation::animate_client;
use crate::backend::x11::X11BackendRef;
use crate::backend::BackendOps;
use crate::client::restore_border_width;
use crate::contexts::{CoreCtx, WmCtx};
use crate::globals::X11RuntimeConfig;
use crate::layouts::arrange;
use crate::types::*;
use x11rb::connection::Connection;

/// Common helper for restoring border width when transitioning to floating.
/// Returns the restored border width value.
fn restore_client_border(core: &mut CoreCtx, backend: &impl BackendOps, win: WindowId) -> i32 {
    restore_border_width(core, win);
    let restored_bw = core
        .g
        .clients
        .get(&win)
        .map(|c| c.border_width)
        .unwrap_or(0);
    BackendOps::set_border_width(backend, win, restored_bw);
    restored_bw
}

/// Apply borderscheme for X11 floating windows (X11 only).
fn apply_floating_borderscheme(x11: &X11BackendRef, win: WindowId, x11_runtime: &X11RuntimeConfig) {
    let pixel = x11_runtime.borderscheme.float_focus.bg.color.pixel;
    let _ = x11rb::protocol::xproto::change_window_attributes(
        x11.conn,
        win.into(),
        &x11rb::protocol::xproto::ChangeWindowAttributesAux::new().border_pixel(Some(pixel as u32)),
    );
}

pub fn set_floating_in_place(ctx: &mut WmCtx, win: WindowId) {
    apply_float_change(ctx, win, true, false, true);
}

pub fn save_floating_win(ctx: &mut WmCtx, win: WindowId) {
    if let Some(client) = ctx.g_mut().clients.get_mut(&win) {
        client.float_geo = client.geo;
    }
}

pub fn restore_floating_win(ctx: &mut WmCtx, win: WindowId) {
    let float_geo = ctx.g().clients.get(&win).map(|c| c.float_geo);
    if let Some(rect) = float_geo {
        crate::client::resize(ctx, win, &rect, false);
    }
}
pub fn apply_float_change(
    ctx: &mut WmCtx,
    win: WindowId,
    floating: bool,
    animate: bool,
    update_borders: bool,
) {
    if floating {
        if let Some(client) = ctx.g_mut().clients.get_mut(&win) {
            client.isfloating = true;
        }

        if update_borders {
            match ctx {
                WmCtx::X11(x11) => {
                    restore_client_border(&mut x11.core, &x11.backend, win);
                    apply_floating_borderscheme(&x11.x11, win, x11.x11_runtime);
                }
                WmCtx::Wayland(wl) => {
                    restore_client_border(&mut wl.core, &wl.backend, win);
                }
            }
        }

        let saved_geo = ctx.g().clients.get(&win).map(|c| c.float_geo);
        let Some(saved_geo) = saved_geo else { return };

        if animate {
            animate_client(ctx, win, &saved_geo, 7, 0);
        } else {
            crate::client::resize(ctx, win, &saved_geo, false);
        }
    } else {
        let client_count = ctx.g().clients.len();
        let clear_border = if let Some(client) = ctx.g_mut().clients.get_mut(&win) {
            client.isfloating = false;
            client.float_geo = client.geo;

            if update_borders && client_count <= 1 && client.snap_status == SnapPosition::None {
                if client.border_width != 0 {
                    client.old_border_width = client.border_width;
                }
                client.border_width = 0;
                true
            } else {
                false
            }
        } else {
            false
        };

        if clear_border {
            ctx.backend().set_border_width(win, 0);
        }
    }
}

pub fn toggle_floating(ctx: &mut WmCtx) {
    let mon = ctx.g().selected_monitor();
    let selected_window = match mon.sel {
        Some(sel) if Some(sel) != mon.overlay => {
            if let Some(c) = ctx.g().clients.get(&sel) {
                if c.is_true_fullscreen() {
                    return;
                }
            }
            Some(sel)
        }
        _ => None,
    };

    let Some(win) = selected_window else { return };

    let (is_floating, is_fixed) = ctx
        .g()
        .clients
        .get(&win)
        .map(|c| (c.isfloating, c.isfixed))
        .unwrap_or((false, false));

    let new_state = !is_floating || is_fixed;
    apply_float_change(ctx, win, new_state, true, true);
    let selmon_id = ctx.g().selected_monitor_id();
    arrange(ctx, Some(selmon_id));
}

pub fn change_floating_win(ctx: &mut WmCtx, win: WindowId) {
    let (_is_fullscreen, is_fake_fullscreen, is_floating, is_fixed) =
        match ctx.g().clients.get(&win) {
            Some(c) => (c.is_fullscreen, c.isfakefullscreen, c.isfloating, c.isfixed),
            None => return,
        };

    if is_fake_fullscreen {
        return;
    }

    let new_state = !is_floating || is_fixed;
    apply_float_change(ctx, win, new_state, false, false);
    let selmon_id = ctx.g().selected_monitor_id();
    arrange(ctx, Some(selmon_id));
}

pub fn set_floating(ctx: &mut WmCtx, win: WindowId, should_arrange: bool) {
    let (is_true_fullscreen, is_floating) = match ctx.g().clients.get(&win) {
        Some(c) => (c.is_true_fullscreen(), c.isfloating),
        None => return,
    };

    if is_true_fullscreen {
        return;
    }
    if is_floating {
        return;
    }

    apply_float_change(ctx, win, true, false, false);

    if should_arrange {
        let selmon_id = ctx.g().selected_monitor_id();
        arrange(ctx, Some(selmon_id));
    }
}

pub fn set_tiled(ctx: &mut WmCtx, win: WindowId, should_arrange: bool) {
    let (is_true_fullscreen, is_floating, is_fixed) = match ctx.client(win) {
        Some(c) => (c.is_true_fullscreen(), c.isfloating, c.isfixed),
        None => return,
    };

    if is_true_fullscreen {
        return;
    }
    if !is_floating && !is_fixed {
        return;
    }

    apply_float_change(ctx, win, false, false, false);

    if should_arrange {
        let selmon_id = ctx.g().selected_monitor_id();
        arrange(ctx, Some(selmon_id));
    }
}

/// Toggle the "maximized" state of the selected window.
///
/// This is a WM-level zoom: the window expands to fill the work area without
/// removing its border or setting `_NET_WM_STATE_FULLSCREEN`.  It is distinct
/// from both real fullscreen (`is_fullscreen`) and fake fullscreen.
///
/// `mon.fullscreen` tracks which window (if any) is currently maximized this
/// way.  Toggling on saves the window's floating geometry so it can be
/// restored on toggle-off.
///
/// Works on both X11 and Wayland.  The X11-specific `apply_size` nudge is
/// only applied on X11, since Wayland geometry is driven by the compositor
/// render loop and needs no such hint.
pub fn toggle_maximized(ctx: &mut WmCtx) {
    // Read all the state we need through the backend-agnostic core.
    let maximized_win = ctx.g().selected_monitor().fullscreen;
    let selected_window = ctx.g().selected_monitor().sel;
    let animated = ctx.g().animated;

    if let Some(win) = maximized_win {
        // --- Exit maximized state ---

        let is_floating = ctx
            .g()
            .clients
            .get(&win)
            .map(|c| c.isfloating)
            .unwrap_or(false);

        // For floating windows (or monitors with no tiling layout), restore
        // the saved pre-maximized geometry.
        if is_floating || !super::helpers::has_tiling_layout(ctx.core()) {
            restore_floating_win(ctx, win);
            // On X11, nudge the window by 1 px so the server re-evaluates
            // size hints and repaints the frame correctly.
            if let WmCtx::X11(x11) = ctx {
                super::helpers::apply_size(&mut x11.core, &x11.x11, win);
            }
        }

        ctx.g_mut().selected_monitor_mut().fullscreen = None;
    } else {
        // --- Enter maximized state ---

        let Some(win) = selected_window else { return };

        ctx.g_mut().selected_monitor_mut().fullscreen = Some(win);

        // Save floating geometry so we can restore it on toggle-off.
        if super::helpers::check_floating(ctx.core(), win) {
            save_floating_win(ctx, win);
        }
    }

    // Run the layout pass.  Disable animations temporarily so the
    // maximize/restore is instantaneous rather than sliding.
    let selmon_id = ctx.g().selected_monitor_id();
    if animated {
        ctx.g_mut().animated = false;
        arrange(ctx, Some(selmon_id));
        ctx.g_mut().animated = true;
    } else {
        arrange(ctx, Some(selmon_id));
    }

    // Raise the newly maximized window above everything else.
    if let Some(win) = ctx.g().selected_monitor().fullscreen {
        ctx.backend().raise_window(win);
    }
}
