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

use crate::constants::animation::EMPHASIZED_FRAME_COUNT;
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::layouts::{arrange, sync_monitor_z_order};
use crate::types::WindowId;

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

        // Signal the application (X11-specific atom write).
        if let WmCtx::X11(ctx_x11) = ctx {
            crate::backend::x11::fullscreen::set_fullscreen_atoms(
                &ctx_x11.x11,
                ctx_x11.x11_runtime,
                win,
                true,
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
                crate::backend::x11::fullscreen::remove_border_x11(&ctx_x11.x11, win);
                ctx.configure_window_geometry(win, mon_rect);
                ctx.raise_window_visual_only(win);
            }
        }

        // Shared: raise the fullscreened window in the monitor z-order.
        sync_monitor_z_order(ctx, monitor_id);
    } else if !fullscreen && mode.is_fullscreen() {
        // ---- Exit fullscreen ------------------------------------------------

        // Backend-specific: clear the fullscreen signal and restore border.
        if let WmCtx::X11(ctx_x11) = ctx {
            crate::backend::x11::fullscreen::set_fullscreen_atoms(
                &ctx_x11.x11,
                ctx_x11.x11_runtime,
                win,
                fullscreen,
            );
            crate::backend::x11::fullscreen::restore_border_x11(
                &ctx_x11.x11,
                &mut ctx_x11.core,
                win,
            );
        }

        crate::client::mode::set_fullscreen(ctx.core_mut().globals_mut(), win, false);

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

pub fn toggle_fake_fullscreen(ctx: &mut WmCtx) {
    match ctx {
        WmCtx::X11(ctx_x11) => crate::backend::x11::fullscreen::toggle_fake_fullscreen_x11(ctx_x11),
        WmCtx::Wayland(_) => {
            if let Some(win) = ctx.core().globals().selected_win() {
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
