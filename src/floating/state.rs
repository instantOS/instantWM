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
/// This is an explicit user/IPC transition: the requested placement becomes
/// immediately visible, so temporary fullscreen and maximized presentations
/// are exited first.
pub fn set_window_mode(
    ctx: &mut WmCtx,
    win: WindowId,
    request: WindowModeRequest,
) -> WindowModeChange {
    let Some(mode) = ctx.core().model().client(win).map(|client| client.mode()) else {
        return WindowModeChange::MissingClient;
    };
    if mode.is_fullscreen() {
        crate::client::fullscreen::set_fullscreen(ctx, win, false);
    }
    if ctx
        .core()
        .model()
        .client(win)
        .is_some_and(|client| client.mode().is_maximized())
    {
        crate::client::fullscreen::leave_maximized(ctx, win);
    }

    let change = set_window_placement_from_policy(ctx, win, request);
    crate::client::fullscreen::sync_client_maximized_signal(ctx, win);
    change
}

/// Change persistent placement without taking ownership of presentation.
///
/// Rules and property refreshes use this path. A policy update may change
/// where a window restores after fullscreen/maximization, but must not cancel
/// a client-owned presentation request.
pub(crate) fn set_window_placement_from_policy(
    ctx: &mut WmCtx,
    win: WindowId,
    request: WindowModeRequest,
) -> WindowModeChange {
    let Some(view) = ctx.core().model().client_view(win) else {
        return WindowModeChange::MissingClient;
    };
    let current_mode = view.client.mode();
    let current_placement = view.client.placement();
    let current_rect = view.client.geo;
    let work_area = view.monitor.work_rect();

    match request {
        WindowModeRequest::Floating(intent) => {
            if current_placement == ClientPlacement::Floating {
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
                client.set_placement(ClientPlacement::Floating);
                if current_mode.is_normal_tiling() {
                    client.restore_border_width();
                } else {
                    client.save_floating_placement(restored_geometry, work_area);
                }
            }

            // Temporary presentation modes retain their current geometry.
            // Only their eventual restore mode and placement change.
            if !current_mode.is_normal_tiling() {
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
            if current_placement == ClientPlacement::Floating
                && let Some(client) = ctx.core_mut().model_mut().client_mut(win)
            {
                if current_mode.is_normal_floating() {
                    client.save_floating_placement(current_rect, work_area);
                }
                client.set_placement(ClientPlacement::Tiling);
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

    let Some((mode, is_fixed)) = ctx
        .core()
        .state()
        .model
        .client(win)
        .map(|c| (c.mode(), c.is_fixed_size))
    else {
        return;
    };
    let request = if mode.placement() != ClientPlacement::Floating || is_fixed {
        WindowModeRequest::Floating(FloatingPlacementIntent::RestoreOrCenter)
    } else {
        WindowModeRequest::Tiling
    };
    let _ = set_window_mode(ctx, win, request);

    let selmon_id = ctx.core().model().selected_monitor_id();
    arrange(ctx, Some(selmon_id));
}

#[cfg(test)]
mod tests {
    use super::{
        WindowModeChange, WindowModeRequest, set_window_mode, set_window_placement_from_policy,
        toggle_floating,
    };
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::client::geometry::FloatingPlacementIntent;
    use crate::types::{Client, ClientMode, ClientPlacement, Monitor, Rect, TagMask, WindowId};
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
        assert!(wm.core.model.attach_client(win));
        wm.core.model.monitor_mut(monitor_id).unwrap().selected = Some(win);
        (wm, win)
    }

    #[test]
    fn tiled_to_floating_applies_and_saves_one_resolved_placement() {
        let (mut wm, win) = wm_with_client(ClientMode::tiled(), Rect::new(0, 30, 1200, 770));

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
        assert_eq!(client.mode(), ClientMode::floating());
        assert_eq!(client.geo, expected);
        assert_eq!(client.saved_floating_rect(), Some(expected));
    }

    #[test]
    fn user_toggle_and_application_maximize_state_stay_bidirectional() {
        let floating = Rect::new(180, 140, 700, 500);
        let (mut wm, win) = wm_with_client(ClientMode::floating(), floating);

        toggle_floating(&mut wm.ctx());
        assert!(wm.core.model.client(win).unwrap().mode().is_normal_tiling());
        assert_eq!(wm.core.model.client_protocol_maximized(win), Some(true));

        toggle_floating(&mut wm.ctx());
        assert!(
            wm.core
                .model
                .client(win)
                .unwrap()
                .mode()
                .is_normal_floating()
        );
        assert_eq!(wm.core.model.client_protocol_maximized(win), Some(false));
        assert_eq!(wm.core.model.client(win).unwrap().geo, floating);
    }

    #[test]
    fn policy_placement_change_does_not_cancel_maximized_presentation() {
        let maximized = Rect::new(0, 30, 1200, 770);
        let (mut wm, win) =
            wm_with_client(ClientMode::maximized(ClientPlacement::Tiling), maximized);
        wm.core.model.client_mut(win).unwrap().border_width = 0;

        let change = set_window_placement_from_policy(
            &mut wm.ctx(),
            win,
            WindowModeRequest::Floating(FloatingPlacementIntent::RestoreOrCenter),
        );

        let client = wm.core.model.client(win).unwrap();
        assert!(matches!(change, WindowModeChange::ChangedToFloating { .. }));
        assert_eq!(
            client.mode(),
            ClientMode::maximized(ClientPlacement::Floating)
        );
        assert_eq!(client.geo, maximized);
        assert_eq!(client.border_width, 0);
        assert_ne!(client.saved_floating_rect(), Some(maximized));
    }

    #[test]
    fn policy_tiling_change_only_changes_maximized_restore_placement() {
        let maximized = Rect::new(0, 30, 1200, 770);
        let (mut wm, win) =
            wm_with_client(ClientMode::maximized(ClientPlacement::Floating), maximized);
        let saved = Rect::new(200, 160, 700, 500);
        wm.core
            .model
            .client_mut(win)
            .unwrap()
            .save_floating_placement(saved, Rect::new(0, 30, 1200, 770));

        let change =
            set_window_placement_from_policy(&mut wm.ctx(), win, WindowModeRequest::Tiling);

        assert_eq!(change, WindowModeChange::ChangedToTiling);
        let client = wm.core.model.client(win).unwrap();
        assert_eq!(
            client.mode(),
            ClientMode::maximized(ClientPlacement::Tiling)
        );
        assert_eq!(client.geo, maximized);
        assert_eq!(client.saved_floating_rect(), Some(saved));
    }

    #[test]
    fn explicit_toggle_from_maximized_floating_becomes_visibly_tiled() {
        let maximized = Rect::new(0, 30, 1200, 770);
        let (mut wm, win) =
            wm_with_client(ClientMode::maximized(ClientPlacement::Floating), maximized);
        let saved = Rect::new(200, 160, 700, 500);
        wm.core
            .model
            .client_mut(win)
            .unwrap()
            .save_floating_placement(saved, Rect::new(0, 30, 1200, 770));

        toggle_floating(&mut wm.ctx());

        let client = wm.core.model.client(win).unwrap();
        assert_eq!(client.mode(), ClientMode::tiled());
        assert_eq!(client.saved_floating_rect(), Some(saved));
    }

    #[test]
    fn explicit_tiling_clears_client_owned_maximization() {
        let maximized = Rect::new(0, 30, 1200, 770);
        let (mut wm, win) =
            wm_with_client(ClientMode::maximized(ClientPlacement::Floating), maximized);
        wm.core
            .model
            .client_mut(win)
            .unwrap()
            .save_floating_placement(Rect::new(200, 160, 700, 500), Rect::new(0, 30, 1200, 770));

        let change = set_window_mode(&mut wm.ctx(), win, WindowModeRequest::Tiling);

        assert_eq!(change, WindowModeChange::ChangedToTiling);
        let client = wm.core.model.client(win).unwrap();
        assert!(client.mode().is_normal_tiling());
        assert_eq!(wm.core.model.client_protocol_maximized(win), Some(true));
    }
}
