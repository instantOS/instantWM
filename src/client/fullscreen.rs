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
/// Native Wayland protocol requests commit the same model transaction
/// synchronously through the compositor command bridge before acknowledging
/// client state.
pub fn set_fullscreen(ctx: &mut WmCtx<'_>, win: WindowId, fullscreen: bool) {
    let Some(transition) = ctx.core_mut().model_mut().set_fullscreen(win, fullscreen) else {
        return;
    };
    let monitor_id = transition.monitor_id();

    match transition {
        crate::client::mode::FullscreenTransition::Unchanged { .. } => {}
        crate::client::mode::FullscreenTransition::EnteredFromLayout { monitor_rect, .. } => {
            apply_fullscreen_signal(ctx, win, true);
            ctx.move_resize(
                win,
                monitor_rect,
                MoveResizeOptions::animate_to(EMPHASIZED_FRAME_COUNT),
            );
            apply_true_fullscreen_backend_effects(ctx, win, monitor_rect);
            sync_monitor_z_order(ctx, monitor_id);
        }
        crate::client::mode::FullscreenTransition::EnteredFromFloating { monitor_rect, .. } => {
            apply_fullscreen_signal(ctx, win, true);
            apply_true_fullscreen_backend_effects(ctx, win, monitor_rect);
            sync_monitor_z_order(ctx, monitor_id);
        }
        crate::client::mode::FullscreenTransition::EnteredFromFakeFullscreen { .. } => {
            apply_fullscreen_signal(ctx, win, true);
            sync_monitor_z_order(ctx, monitor_id);
        }
        crate::client::mode::FullscreenTransition::ExitedToFloating { restore_rect, .. } => {
            apply_fullscreen_exit_backend_effects(ctx, win);
            ctx.move_resize(win, restore_rect, MoveResizeOptions::immediate());
            arrange(ctx, Some(monitor_id));
        }
        crate::client::mode::FullscreenTransition::ExitedToTiling { .. } => {
            apply_fullscreen_exit_backend_effects(ctx, win);
            arrange(ctx, Some(monitor_id));
        }
        crate::client::mode::FullscreenTransition::ExitedFakeFullscreen { .. } => {
            apply_fullscreen_exit_backend_effects(ctx, win);
            sync_monitor_z_order(ctx, monitor_id);
        }
    }
}

fn apply_true_fullscreen_backend_effects(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    monitor_rect: crate::types::Rect,
) {
    if let WmCtx::X11(ctx_x11) = ctx {
        crate::backend::x11::fullscreen::remove_border(&ctx_x11.x11, win);
        ctx_x11.x11.configure_window_geometry(win, monitor_rect);
        ctx_x11.x11.raise_window_visual_only(win);
    }
}

fn apply_fullscreen_exit_backend_effects(ctx: &mut WmCtx<'_>, win: WindowId) {
    apply_fullscreen_signal(ctx, win, false);
    if let WmCtx::X11(ctx_x11) = ctx {
        crate::backend::x11::fullscreen::restore_border(&ctx_x11.x11, ctx_x11.core.model(), win);
    }
}

fn apply_fullscreen_signal(ctx: &mut WmCtx<'_>, win: WindowId, fullscreen: bool) {
    if let WmCtx::X11(ctx_x11) = ctx {
        crate::backend::x11::fullscreen::set_fullscreen_atoms(
            &ctx_x11.x11,
            ctx_x11.x11_runtime,
            win,
            fullscreen,
        );
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
