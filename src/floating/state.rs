//! Floating state transitions and geometry persistence.

use crate::backend::BackendOps;
use crate::backend::x11::X11BackendRef;
use crate::client::restore_border_width;
use crate::constants::animation::DEFAULT_FRAME_COUNT;
use crate::contexts::{CoreCtx, WmCtx};
use crate::geometry::MoveResizeOptions;
use crate::layouts::arrange;
use crate::types::*;

/// Whether a window should be floating or tiled.
#[derive(Clone, Copy, Default)]
pub enum WindowMode {
    #[default]
    Tiled,
    Floating,
    /// Toggle between floating and tiled based on current state.
    ToggleFloat,
}

/// Common helper for restoring border width when transitioning to floating.
/// Returns the restored border width value.
/// This is X11-specific since Wayland doesn't support border widths.
fn restore_client_border(core: &mut CoreCtx, x11: &X11BackendRef<'_>, win: WindowId) -> i32 {
    if let Some(client) = core.globals_mut().clients.get_mut(&win) {
        restore_border_width(client);
    }
    let restored_bw = core
        .globals()
        .clients
        .get(&win)
        .map(|c| c.border_width)
        .unwrap_or(0);
    x11.set_border_width(win, restored_bw);
    restored_bw
}

pub fn save_floating_geometry(client: &mut Client) {
    client.float_geo = client.geo;
}

pub fn restore_floating_geometry(ctx: &mut WmCtx, win: WindowId) {
    if let Some(rect) = ctx.core().globals().clients.effective_float_geo(win) {
        ctx.move_resize(win, rect, MoveResizeOptions::hinted_immediate(false));
    }
}

/// Set a window to floating or tiled mode.
/// Returns true if the caller should animate (when going to floating mode).
/// Handles border updates and geometry changes but NOT animation (callers handle that separately).
pub fn set_window_mode(ctx: &mut WmCtx, win: WindowId, mode: WindowMode) -> bool {
    // Handle ToggleFloat by determining the actual target mode
    let target_mode = match mode {
        WindowMode::ToggleFloat => {
            let (is_floating, is_fixed) = ctx
                .client(win)
                .map(|c| (c.is_floating, c.is_fixed_size))
                .unwrap_or((false, false));
            // If currently tiled, go floating; if floating, go tiled (unless fixed)
            if !is_floating || is_fixed {
                WindowMode::Floating
            } else {
                WindowMode::Tiled
            }
        }
        other => other,
    };

    match target_mode {
        WindowMode::Floating => {
            if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
                client.is_floating = true;
            }

            // Restore borders
            match ctx {
                WmCtx::X11(x11) => {
                    restore_client_border(&mut x11.core, &x11.x11, win);
                    crate::backend::x11::floating::apply_floating_borderscheme(
                        &x11.x11,
                        win,
                        x11.x11_runtime,
                    );
                }
                WmCtx::Wayland(_) => {
                    // Wayland doesn't support border widths, nothing to restore
                }
            }

            // Apply saved float geometry
            let saved_geo = ctx.core().globals().clients.effective_float_geo(win);
            let Some(saved_geo) = saved_geo else {
                return false;
            };
            ctx.move_resize(win, saved_geo, MoveResizeOptions::hinted_immediate(false));
            true // Caller should animate
        }
        WindowMode::Tiled => {
            let client_count = ctx.core().globals().clients.len();
            let clear_border = if let Some(client) = ctx.client_mut(win) {
                client.is_floating = false;
                client.float_geo = client.geo;

                // Only clear border if this is the only client and not snapped
                if client_count <= 1 && client.snap_status == SnapPosition::None {
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

            // Border width clearing is X11-specific
            if clear_border && let WmCtx::X11(x11) = ctx {
                x11.x11.set_border_width(win, 0);
            }
            false // No animation needed for tiling
        }
        WindowMode::ToggleFloat => unreachable!(), // Already handled above
    }
}

pub fn toggle_floating(ctx: &mut WmCtx) {
    let mon = ctx.core().globals().selected_monitor();
    let selected_window = match mon.sel {
        Some(sel) if Some(sel) != mon.overlay => {
            if let Some(c) = ctx.client(sel)
                && c.is_true_fullscreen()
            {
                return;
            }
            Some(sel)
        }
        _ => None,
    };

    let Some(win) = selected_window else { return };

    // ToggleFloat handles deriving current state and determining new state internally
    let should_animate = set_window_mode(ctx, win, WindowMode::ToggleFloat);

    // Animate when going to floating mode
    if should_animate && let Some(saved_geo) = ctx.core().globals().clients.effective_float_geo(win)
    {
        ctx.move_resize(
            win,
            saved_geo,
            MoveResizeOptions::animate_to(DEFAULT_FRAME_COUNT),
        );
    }

    let selmon_id = ctx.core().globals().selected_monitor_id();
    arrange(ctx, Some(selmon_id));
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
    let maximized_win = ctx.core().globals().selected_monitor().fullscreen;
    let selected_window = ctx.selected_client();
    let animated = ctx.core().globals().behavior.animated;

    if let Some(win) = maximized_win {
        // --- Exit maximized state ---

        let is_floating = ctx
            .core()
            .globals()
            .clients
            .get(&win)
            .map(|c| c.is_floating)
            .unwrap_or(false);

        // For floating windows (or monitors with no tiling layout), restore
        // the saved pre-maximized geometry.
        if is_floating || !super::helpers::has_tiling_layout(ctx.core()) {
            restore_floating_geometry(ctx, win);
            // On X11, nudge the window by 1 px so the server re-evaluates
            // size hints and repaints the frame correctly.
            if let WmCtx::X11(x11) = ctx {
                super::helpers::apply_size(x11, win);
            }
        }

        ctx.core_mut()
            .globals_mut()
            .selected_monitor_mut()
            .fullscreen = None;
    } else {
        // --- Enter maximized state ---

        let Some(win) = selected_window else { return };

        ctx.core_mut()
            .globals_mut()
            .selected_monitor_mut()
            .fullscreen = Some(win);

        // Save floating geometry so we can restore it on toggle-off.
        if super::helpers::check_floating(ctx.core(), win)
            && let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win)
        {
            save_floating_geometry(client);
        }
    }

    // Run the layout pass.  Disable animations temporarily so the
    // maximize/restore is instantaneous rather than sliding.
    let selmon_id = ctx.core().globals().selected_monitor_id();
    if animated {
        ctx.core_mut().globals_mut().behavior.animated = false;
        arrange(ctx, Some(selmon_id));
        ctx.core_mut().globals_mut().behavior.animated = true;
    } else {
        arrange(ctx, Some(selmon_id));
    }

    // Raise the newly maximized window above everything else.
    if let Some(win) = ctx.core().globals().selected_monitor().fullscreen {
        ctx.backend().raise_window_visual_only(win);
    }
}
