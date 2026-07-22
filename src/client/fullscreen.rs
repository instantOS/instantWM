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

use crate::backend::WindowOps;
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
    let Some(transition) = ctx.core_mut().model_mut().set_fullscreen(win, fullscreen) else {
        return;
    };
    if !transition.changed() {
        return;
    }

    if transition.entered() {
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

        if !transition.was_fake_fullscreen() {
            if !transition.was_floating() {
                ctx.move_resize(
                    win,
                    transition.monitor_rect(),
                    MoveResizeOptions::animate_to(EMPHASIZED_FRAME_COUNT),
                );
            }

            // Backend-specific: remove border, enforce geometry, raise.
            if let WmCtx::X11(ctx_x11) = ctx {
                crate::backend::x11::fullscreen::remove_border(&ctx_x11.x11, win);
                ctx_x11
                    .x11
                    .configure_window_geometry(win, transition.monitor_rect());
                ctx_x11.x11.raise_window_visual_only(win);
            }
        }

        // Shared: raise the fullscreened window in the monitor z-order.
        sync_monitor_z_order(ctx, transition.monitor_id());
    } else if transition.exited() {
        // ---- Exit fullscreen ------------------------------------------------

        // Backend-specific: clear the fullscreen signal and restore border.
        if let WmCtx::X11(ctx_x11) = ctx {
            crate::backend::x11::fullscreen::set_fullscreen_atoms(
                &ctx_x11.x11,
                ctx_x11.x11_runtime,
                win,
                fullscreen,
            );
            crate::backend::x11::fullscreen::restore_border(
                &ctx_x11.x11,
                ctx_x11.core.model(),
                win,
            );
        }

        // Shared: restore old geometry and re-layout.
        if !transition.was_fake_fullscreen() {
            ctx.move_resize(win, transition.old_geo(), MoveResizeOptions::immediate());
            arrange(ctx, Some(transition.monitor_id()));
        } else {
            sync_monitor_z_order(ctx, transition.monitor_id());
        }
    }
}

// ---------------------------------------------------------------------------
// Fake fullscreen toggle
// ---------------------------------------------------------------------------

pub fn toggle_fake_fullscreen(ctx: &mut WmCtx) {
    match ctx {
        WmCtx::X11(ctx_x11) => crate::backend::x11::fullscreen::toggle_fake_fullscreen(ctx_x11),
        WmCtx::Wayland(_) => {
            if let Some(win) = ctx.core().model().selected_win() {
                if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
                    if client.mode().is_fake_fullscreen() {
                        client.restore_mode();
                    } else {
                        client.enter_fake_fullscreen();
                    }
                }
                let selmon_id = ctx.core().model().selected_monitor_id();
                ctx.core_mut().queue_layout_for_monitor_urgent(selmon_id);
            }
        }
    }
}
