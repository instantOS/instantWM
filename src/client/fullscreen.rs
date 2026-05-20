//! Fullscreen and fake-fullscreen state management.
//!
//! # Responsibilities
//!
//! * [`set_fullscreen`]         – enter/exit real fullscreen, updating
//!   `_NET_WM_STATE` and animating the transition.
//! * [`toggle_fake_fullscreen`] – toggle "fake" fullscreen (window fills the
//!   monitor but still participates in the layout).
//! * [`save_border_width`]      – snapshot the current border width before
//!   entering fullscreen.
//! * [`restore_border_width`]   – reinstate the saved border width on exit.
//!
//! ## Real vs. fake fullscreen
//!
//! *Real* fullscreen:
//! the border is removed, the window is raised above everything else, and it
//! is resized to exactly the monitor rectangle.
//!
//! *Fake* fullscreen:
//! the `_NET_WM_STATE_FULLSCREEN` atom is set (so the application thinks it is
//! fullscreen) but the window remains in the normal layout stack with its
//! border intact.

use crate::backend::x11::properties::{get_atom_props, write_net_wm_state_atoms};
use crate::constants::animation::EMPHASIZED_FRAME_COUNT;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::geometry::MoveResizeOptions;
use crate::layouts::{arrange, sync_monitor_z_order};
use crate::types::{ClientMode, Rect, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;

// ---------------------------------------------------------------------------
// Real fullscreen
// ---------------------------------------------------------------------------

/// Backend‑agnostic entry point: enter or exit real fullscreen for `win`.
///
/// Handles shared state (mode, layout, z‑order) and delegates X11‑specific
/// protocol work (atoms, `configure_window`) inline.
///
/// For the Wayland backend the compositor owns the fullscreen geometry and
/// stacking, so this function just updates the mode and queues a layout.
pub fn set_fullscreen(ctx: &mut WmCtx<'_>, win: WindowId, fullscreen: bool) {
    let snapshot = ctx
        .core()
        .globals()
        .clients
        .get(&win)
        .map(|c| (c.mode, c.monitor_id, c.old_geo));
    let Some((mode, monitor_id, old_geo)) = snapshot else {
        return;
    };

    if fullscreen && !mode.is_fullscreen() {
        // ---- Enter fullscreen -----------------------------------------------

        // Backend-specific: signal the application.
        if let WmCtx::X11(ctx_x11) = ctx {
            let x11_win: Window = win.into();
            let mut state = get_atom_props(
                ctx_x11.x11.conn,
                x11_win,
                ctx_x11.x11_runtime.netatom.wm_state,
            );
            let fullscreen_atom = ctx_x11.x11_runtime.netatom.wm_fullscreen;
            if !state.contains(&fullscreen_atom) {
                state.push(fullscreen_atom);
            }
            write_net_wm_state_atoms(
                ctx_x11.x11.conn,
                x11_win,
                ctx_x11.x11_runtime.netatom.wm_state,
                &state,
            );
        }

        // Shared: save border width, flip client mode.
        let outcome = crate::client::mode::set_fullscreen(ctx.core_mut().globals_mut(), win, true);

        if let Some(crate::client::mode::FullscreenOutcome::Entered { was_floating }) = outcome
            && !mode.is_fake_fullscreen()
        {
            let mon_rect = ctx
                .core()
                .globals()
                .monitor(monitor_id)
                .map(|m| m.monitor_rect)
                .unwrap_or_default();

            if !was_floating {
                ctx.move_resize(
                    win,
                    mon_rect,
                    MoveResizeOptions::animate_to(EMPHASIZED_FRAME_COUNT),
                );
            }

            // Backend-specific: remove border, enforce geometry, raise.
            if let WmCtx::X11(ctx_x11) = ctx {
                let x11_win: Window = win.into();
                let _ = ctx_x11
                    .x11
                    .conn
                    .configure_window(x11_win, &ConfigureWindowAux::new().border_width(0));
                let _ = ctx_x11.x11.conn.configure_window(
                    x11_win,
                    &ConfigureWindowAux::new()
                        .x(mon_rect.x)
                        .y(mon_rect.y)
                        .width(mon_rect.w as u32)
                        .height(mon_rect.h as u32),
                );
                let _ = ctx_x11.x11.conn.configure_window(
                    x11_win,
                    &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                );
                let _ = ctx_x11.x11.conn.flush();
            }
        }

        // Shared: raise the fullscreened window in the monitor z-order.
        sync_monitor_z_order(ctx, monitor_id);
    } else if !fullscreen && mode.is_fullscreen() {
        // ---- Exit fullscreen ------------------------------------------------

        // Backend-specific: clear the fullscreen signal.
        if let WmCtx::X11(ctx_x11) = ctx {
            let x11_win: Window = win.into();
            let mut state = get_atom_props(
                ctx_x11.x11.conn,
                x11_win,
                ctx_x11.x11_runtime.netatom.wm_state,
            );
            state.retain(|&a| a != ctx_x11.x11_runtime.netatom.wm_fullscreen);
            write_net_wm_state_atoms(
                ctx_x11.x11.conn,
                x11_win,
                ctx_x11.x11_runtime.netatom.wm_state,
                &state,
            );
        }

        // Shared: restore client mode and border width.
        crate::client::mode::set_fullscreen(ctx.core_mut().globals_mut(), win, false);

        // Backend-specific: reinstate the X11 border.
        if let WmCtx::X11(ctx_x11) = ctx {
            let x11_win: Window = win.into();
            let restored_border = ctx_x11
                .core
                .client(win)
                .map(|c| c.border_width.max(0) as u32)
                .unwrap_or(0);
            let _ = ctx_x11.x11.conn.configure_window(
                x11_win,
                &ConfigureWindowAux::new().border_width(restored_border),
            );
        }

        // Shared: restore old geometry and re-layout.
        if !mode.is_fake_fullscreen() {
            ctx.move_resize(win, old_geo, MoveResizeOptions::immediate());
            arrange(ctx, Some(monitor_id));
        } else {
            sync_monitor_z_order(ctx, monitor_id);
        }
    }
}

// ---------------------------------------------------------------------------
// Fake fullscreen toggle
// ---------------------------------------------------------------------------

/// Toggle the "fake fullscreen" mode on the currently selected window.
pub fn toggle_fake_fullscreen_x11(ctx_x11: &mut WmCtxX11<'_>) {
    let Some(win) = ctx_x11.core.selected_client() else {
        return;
    };

    let (mode, monitor_id, old_border_width) = ctx_x11
        .core
        .globals()
        .clients
        .get(&win)
        .map(|c| (c.mode, c.monitor_id, c.old_border_width))
        .unwrap_or((ClientMode::Tiling, crate::types::MonitorId(0), 0));

    // Transitioning from fake-fullscreen → real-fullscreen: resize to fill the
    // monitor and raise the window.
    if mode.is_fake_fullscreen() {
        let borderpx = ctx_x11.core.globals().cfg.window.border_width_px;

        let mon_rect = ctx_x11
            .core
            .globals()
            .monitor(monitor_id)
            .map(|m| m.monitor_rect)
            .unwrap_or_default();

        let mut wm_ctx = WmCtx::X11(ctx_x11.reborrow());
        wm_ctx.move_resize(
            win,
            Rect {
                x: mon_rect.x + borderpx,
                y: mon_rect.y + borderpx,
                w: mon_rect.w - 2 * borderpx,
                h: mon_rect.h - 2 * borderpx,
            },
            MoveResizeOptions::immediate(),
        );

        let x11_win: Window = win.into();
        let _ = ctx_x11.x11.conn.configure_window(
            x11_win,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
        let _ = ctx_x11.x11.conn.flush();
    }

    // Restore the border width when leaving fake-fullscreen while still in
    // the fullscreen state (real fullscreen removes the border, so we need to
    // put it back before the layout re-runs).
    if let Some(client) = ctx_x11.core.globals_mut().clients.get_mut(&win) {
        match client.mode {
            ClientMode::FakeFullscreen { .. } => client.mode = client.mode.as_fullscreen(),
            ClientMode::TrueFullscreen { .. } => client.mode = client.mode.as_fake_fullscreen(),
            _ => client.mode = client.mode.as_fake_fullscreen(),
        }
        client.border_width = if client.mode.is_true_fullscreen() {
            0
        } else {
            old_border_width
        };
    }
}

pub fn toggle_fake_fullscreen(ctx: &mut WmCtx) {
    match ctx {
        WmCtx::X11(ctx_x11) => toggle_fake_fullscreen_x11(ctx_x11),
        WmCtx::Wayland(_) => {
            if let Some(win) = ctx.core().selected_client() {
                if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
                    if client.mode.is_fake_fullscreen() {
                        client.mode = client.mode.restored();
                    } else {
                        client.mode = client.mode.as_fake_fullscreen();
                    }
                }
                let selmon_id = ctx.core().globals().selected_monitor_id();
                ctx.core_mut()
                    .globals_mut()
                    .queue_layout_for_monitor_urgent(selmon_id);
            }
        }
    }
}
