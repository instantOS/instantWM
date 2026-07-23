//! Floating state transitions and geometry persistence.

use crate::client::geometry::{FloatingPlacementIntent, resolve_floating_transition};
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::layouts::arrange;
use crate::types::*;

pub fn restore_floating_geometry(ctx: &mut WmCtx, win: WindowId) {
    let Some(view) = ctx.core().model().client_view(win) else {
        return;
    };
    let rect = resolve_floating_transition(
        view.client,
        view.monitor.work_rect(),
        FloatingPlacementIntent::RestoreOrCenter,
    );
    ctx.move_resize(win, rect, MoveResizeOptions::for_floating_transition());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum WindowModeChange {
    MissingClient,
    ChangedToFloating { restored_geometry: Rect },
    ChangedToTiling,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowModeRequest {
    Floating(FloatingPlacementIntent),
    Tiling,
}

/// Set a window to floating or tiled mode.
///
/// Handles border updates and geometry changes but not caller-owned animation.
pub fn set_window_mode(
    ctx: &mut WmCtx,
    win: WindowId,
    request: WindowModeRequest,
) -> WindowModeChange {
    let Some(view) = ctx.core().model().client_view(win) else {
        return WindowModeChange::MissingClient;
    };
    let current_mode = view.client.mode();
    let current_base_mode = view.client.base_mode();
    let current_rect = view.client.geo;
    let work_area = view.monitor.work_rect();

    match request {
        WindowModeRequest::Floating(intent) => {
            if current_base_mode == BaseClientMode::Floating {
                return WindowModeChange::ChangedToFloating {
                    restored_geometry: current_rect,
                };
            }

            let mut placement_client = view.client.clone();
            placement_client.restore_border_width();
            let restored_geometry =
                resolve_floating_transition(&placement_client, work_area, intent);
            let border_width = placement_client.border_width;

            if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
                client.set_base_mode(BaseClientMode::Floating);
                if current_mode.is_tiling() {
                    client.restore_border_width();
                } else {
                    client.save_floating_placement(restored_geometry, work_area);
                }
            }

            // Temporary presentation modes retain their current geometry.
            // Only their eventual restore mode and placement change.
            if !current_mode.is_tiling() {
                return WindowModeChange::ChangedToFloating { restored_geometry };
            }
            if let WmCtx::X11(x11) = ctx {
                x11.x11.set_border_width(win, 0);
                x11.x11.set_border_width(win, border_width);
                crate::backend::x11::floating::apply_floating_borderscheme(
                    &x11.x11,
                    win,
                    x11.x11_runtime,
                );
            }

            ctx.move_resize(
                win,
                restored_geometry,
                MoveResizeOptions::for_floating_transition(),
            );
            WindowModeChange::ChangedToFloating { restored_geometry }
        }
        WindowModeRequest::Tiling => {
            if current_base_mode == BaseClientMode::Floating
                && let Some(client) = ctx.core_mut().model_mut().client_mut(win)
            {
                if current_mode.is_floating() {
                    client.save_floating_placement(current_rect, work_area);
                }
                client.set_base_mode(BaseClientMode::Tiling);
            }
            WindowModeChange::ChangedToTiling
        }
    }
}

pub fn toggle_floating(ctx: &mut WmCtx) {
    let mon = ctx.core().model().expect_selected_monitor();
    let selected_window = match mon.selected {
        Some(sel)
            if !ctx
                .core()
                .state()
                .model
                .client(sel)
                .is_some_and(|c| c.is_edge_scratchpad()) =>
        {
            if let Some(c) = ctx.core().model().client(sel)
                && c.mode().is_true_fullscreen()
            {
                return;
            }
            Some(sel)
        }
        _ => None,
    };

    let Some(win) = selected_window else { return };

    let Some((base_mode, is_fixed)) = ctx
        .core()
        .state()
        .model
        .client(win)
        .map(|c| (c.base_mode(), c.is_fixed_size))
    else {
        return;
    };
    let request = if base_mode != BaseClientMode::Floating || is_fixed {
        WindowModeRequest::Floating(FloatingPlacementIntent::RestoreOrCenter)
    } else {
        WindowModeRequest::Tiling
    };
    let _ = set_window_mode(ctx, win, request);

    let selmon_id = ctx.core().model().selected_monitor_id();
    arrange(ctx, Some(selmon_id));
}

/// Toggle the "maximized" state of the selected window.
///
/// This is a WM-level zoom: the window expands to fill the work area without
/// removing its border or setting `_NET_WM_STATE_FULLSCREEN`.  It is distinct
/// from both real fullscreen and fake fullscreen.
///
/// `maximized_client` derives which window (if any) is currently maximized
/// this way from the clients' modes.  Toggling on saves the window's floating
/// geometry so it can be restored on toggle-off.
///
/// Works on both X11 and Wayland.  The X11-specific `apply_size` nudge is
/// only applied on X11, since Wayland geometry is driven by the compositor
/// render loop and needs no such hint.
pub(crate) fn toggle_client_maximized(ctx: &mut WmCtx) {
    let maximized_win = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .maximized_client(&ctx.core().model().clients);
    let selected_window = ctx.core().model().selected_win();
    let animated = ctx.core().behavior().animated;

    let enter = maximized_win.is_none();
    let win = if enter {
        selected_window
    } else {
        maximized_win
    };
    let Some(win) = win else { return };

    let Some(transition) = ctx.core_mut().model_mut().set_maximized(win, enter) else {
        return;
    };
    let entered = transition.entered();

    if transition.exited()
        && (transition.restore_base() == BaseClientMode::Floating
            || !super::helpers::has_tiling_layout(ctx.core().model()))
    {
        ctx.move_resize(
            win,
            transition.restore_rect(),
            MoveResizeOptions::hinted_immediate(false),
        );
        if let WmCtx::X11(x11) = ctx {
            super::helpers::apply_size(x11, win);
        }
    }

    // Run the layout pass.  Disable animations temporarily so the
    // maximize/restore is instantaneous rather than sliding.
    let monitor_id = transition.monitor_id();
    if animated {
        ctx.core_mut().behavior_mut().animated = false;
        arrange(ctx, Some(monitor_id));
        ctx.core_mut().behavior_mut().animated = true;
    } else {
        arrange(ctx, Some(monitor_id));
    }

    // Raise the newly maximized window above everything else.
    if entered {
        ctx.raise_client(win);
    }
}

#[cfg(test)]
mod tests {
    use super::{WindowModeChange, WindowModeRequest, set_window_mode};
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::client::geometry::FloatingPlacementIntent;
    use crate::types::{BaseClientMode, Client, ClientMode, Monitor, Rect, TagMask, WindowId};
    use crate::wm::Wm;

    fn wm_with_client(mode: ClientMode, geo: Rect) -> (Wm, WindowId) {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1200, 800),
            available_rect: Rect::new(0, 30, 1200, 770),
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        let win = WindowId(91);
        wm.core.model.insert_client(Client {
            win,
            monitor_id,
            tags: TagMask::single(1).unwrap(),
            mode,
            geo,
            border_width: 2,
            old_border_width: 2,
            ..Client::default()
        });
        (wm, win)
    }

    #[test]
    fn tiled_to_floating_applies_and_saves_one_resolved_placement() {
        let (mut wm, win) = wm_with_client(ClientMode::Tiling, Rect::new(0, 30, 1200, 770));

        let change = set_window_mode(
            &mut wm.ctx(),
            win,
            WindowModeRequest::Floating(FloatingPlacementIntent::RestoreOrCenter),
        );

        let expected = Rect::new(150, 126, 896, 573);
        assert_eq!(
            change,
            WindowModeChange::ChangedToFloating {
                restored_geometry: expected
            }
        );
        let client = wm.core.model.client(win).unwrap();
        assert_eq!(client.mode(), ClientMode::Floating);
        assert_eq!(client.geo, expected);
        assert_eq!(client.saved_floating_rect(), Some(expected));
    }

    #[test]
    fn changing_the_base_mode_under_maximize_does_not_resize_presentation() {
        let maximized = Rect::new(0, 30, 1200, 770);
        let (mut wm, win) = wm_with_client(
            ClientMode::Maximized {
                restore: BaseClientMode::Tiling,
            },
            maximized,
        );
        wm.core.model.client_mut(win).unwrap().border_width = 0;

        let change = set_window_mode(
            &mut wm.ctx(),
            win,
            WindowModeRequest::Floating(FloatingPlacementIntent::RestoreOrCenter),
        );

        let client = wm.core.model.client(win).unwrap();
        assert!(matches!(change, WindowModeChange::ChangedToFloating { .. }));
        assert_eq!(
            client.mode(),
            ClientMode::Maximized {
                restore: BaseClientMode::Floating
            }
        );
        assert_eq!(client.geo, maximized);
        assert_eq!(client.border_width, 0);
        assert_ne!(client.saved_floating_rect(), Some(maximized));
    }

    #[test]
    fn tiling_request_under_maximize_changes_only_the_restore_mode() {
        let maximized = Rect::new(0, 30, 1200, 770);
        let (mut wm, win) = wm_with_client(
            ClientMode::Maximized {
                restore: BaseClientMode::Floating,
            },
            maximized,
        );
        let saved = Rect::new(200, 160, 700, 500);
        wm.core
            .model
            .client_mut(win)
            .unwrap()
            .save_floating_placement(saved, Rect::new(0, 30, 1200, 770));

        let change = set_window_mode(&mut wm.ctx(), win, WindowModeRequest::Tiling);

        assert_eq!(change, WindowModeChange::ChangedToTiling);
        let client = wm.core.model.client(win).unwrap();
        assert_eq!(
            client.mode(),
            ClientMode::Maximized {
                restore: BaseClientMode::Tiling
            }
        );
        assert_eq!(client.geo, maximized);
        assert_eq!(client.saved_floating_rect(), Some(saved));
    }
}
