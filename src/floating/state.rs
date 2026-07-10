//! Floating state transitions and geometry persistence.

use crate::constants::animation::DEFAULT_FRAME_COUNT;
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::layouts::arrange;
use crate::types::*;

pub fn restore_floating_geometry(ctx: &mut WmCtx, win: WindowId) {
    if let Some(rect) = ctx.core().model().clients.effective_float_geo(win) {
        ctx.move_resize(win, rect, MoveResizeOptions::hinted_immediate(false));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum WindowModeChange {
    MissingClient,
    ChangedToFloating { restored_geometry: bool },
    ChangedToTiling,
}

impl WindowModeChange {
    pub fn should_animate_float_restore(self) -> bool {
        matches!(
            self,
            WindowModeChange::ChangedToFloating {
                restored_geometry: true
            }
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WindowModePlan {
    pub(crate) change: WindowModeChange,
    pub(crate) border_width: i32,
    pub(crate) restore_geometry: Option<Rect>,
}

pub(crate) fn update_window_mode(
    clients: &mut crate::client::manager::ClientManager,
    win: WindowId,
    mode: BaseClientMode,
) -> Option<WindowModePlan> {
    let client = clients.get_mut(&win)?;
    match mode {
        BaseClientMode::Floating => {
            client.mode = ClientMode::Floating;
            client.restore_border_width();
            let border_width = client.border_width;
            let restore_geometry = Some(client.effective_float_geo());
            Some(WindowModePlan {
                change: WindowModeChange::ChangedToFloating {
                    restored_geometry: restore_geometry.is_some(),
                },
                border_width,
                restore_geometry,
            })
        }
        BaseClientMode::Tiling => {
            client.enter_tiling();
            Some(WindowModePlan {
                change: WindowModeChange::ChangedToTiling,
                border_width: client.border_width,
                restore_geometry: None,
            })
        }
    }
}

/// Set a window to floating or tiled mode.
///
/// Handles border updates and geometry changes but not caller-owned animation.
pub fn set_window_mode(ctx: &mut WmCtx, win: WindowId, mode: BaseClientMode) -> WindowModeChange {
    let Some(plan) = update_window_mode(&mut ctx.core_mut().model_mut().clients, win, mode) else {
        return WindowModeChange::MissingClient;
    };

    match mode {
        BaseClientMode::Floating => {
            if let WmCtx::X11(x11) = ctx {
                x11.x11.set_border_width(win, 0);
                x11.x11.set_border_width(win, plan.border_width);
                crate::backend::x11::floating::apply_floating_borderscheme(
                    &x11.x11,
                    win,
                    x11.x11_runtime,
                );
            }

            // Apply saved float geometry
            if let Some(saved_geo) = plan.restore_geometry {
                ctx.move_resize(win, saved_geo, MoveResizeOptions::hinted_immediate(false));
            }
            plan.change
        }
        BaseClientMode::Tiling => plan.change,
    }
}

pub fn toggle_floating(ctx: &mut WmCtx) {
    let mon = ctx.core().model().selected_monitor();
    let selected_window = match mon.selected {
        Some(sel)
            if !ctx
                .core()
                .state()
                .model
                .clients
                .get(&sel)
                .is_some_and(|c| c.is_edge_scratchpad()) =>
        {
            if let Some(c) = ctx.core().model().clients.get(&sel)
                && c.mode.is_true_fullscreen()
            {
                return;
            }
            Some(sel)
        }
        _ => None,
    };

    let Some(win) = selected_window else { return };

    let (is_floating, is_fixed) = ctx
        .core()
        .state()
        .model
        .clients
        .get(&win)
        .map(|c| (c.mode.is_floating(), c.is_fixed_size))
        .unwrap_or((false, false));
    let target_mode = if !is_floating || is_fixed {
        BaseClientMode::Floating
    } else {
        BaseClientMode::Tiling
    };
    let mode_change = set_window_mode(ctx, win, target_mode);

    // Animate when going to floating mode
    if mode_change.should_animate_float_restore()
        && let Some(saved_geo) = ctx.core().model().clients.effective_float_geo(win)
    {
        ctx.move_resize(
            win,
            saved_geo,
            MoveResizeOptions::animate_to(DEFAULT_FRAME_COUNT),
        );
    }

    let selmon_id = ctx.core().model().selected_monitor_id();
    arrange(ctx, Some(selmon_id));
}

/// Toggle the "maximized" state of the selected window.
///
/// This is a WM-level zoom: the window expands to fill the work area without
/// removing its border or setting `_NET_WM_STATE_FULLSCREEN`.  It is distinct
/// from both real fullscreen and fake fullscreen.
///
/// `mon.maximized` tracks which window (if any) is currently maximized this
/// way.  Toggling on saves the window's floating geometry so it can be
/// restored on toggle-off.
///
/// Works on both X11 and Wayland.  The X11-specific `apply_size` nudge is
/// only applied on X11, since Wayland geometry is driven by the compositor
/// render loop and needs no such hint.
pub fn toggle_maximized(ctx: &mut WmCtx) {
    let maximized_win = ctx.core().model().selected_monitor().maximized;
    let selected_window = ctx.core().model().selected_win();
    let animated = ctx.core().behavior().animated;

    let enter = maximized_win.is_none();
    let win = if enter {
        selected_window
    } else {
        maximized_win
    };
    let Some(win) = win else { return };

    let outcome = crate::client::mode::set_maximized(ctx.core_mut().model_mut(), win, enter);

    if let Some(crate::client::mode::MaximizedOutcome::Exited { base }) = outcome
        && (base == BaseClientMode::Floating
            || !super::helpers::has_tiling_layout(ctx.core().model()))
    {
        restore_floating_geometry(ctx, win);
        if let WmCtx::X11(x11) = ctx {
            super::helpers::apply_size(x11, win);
        }
    }

    // Run the layout pass.  Disable animations temporarily so the
    // maximize/restore is instantaneous rather than sliding.
    let selmon_id = ctx.core().model().selected_monitor_id();
    if animated {
        ctx.core_mut().behavior_mut().animated = false;
        arrange(ctx, Some(selmon_id));
        ctx.core_mut().behavior_mut().animated = true;
    } else {
        arrange(ctx, Some(selmon_id));
    }

    // Raise the newly maximized window above everything else.
    if ctx.core().model().selected_monitor().maximized == Some(win) {
        ctx.window_backend().raise_window_visual_only(win);
    }
}

#[cfg(test)]
mod tests {
    use super::WindowModeChange;
    use super::update_window_mode;
    use crate::client::manager::ClientManager;
    use crate::types::*;

    fn make_client(win: WindowId, tags: TagMask, mode: ClientMode) -> Client {
        Client {
            win,
            tags,
            monitor_id: MonitorId::default(),
            mode,
            geo: Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 100,
            },
            float_geo: Rect {
                x: 10,
                y: 10,
                w: 200,
                h: 200,
            },
            border_width: 2,
            old_border_width: 4,
            ..Client::default()
        }
    }

    fn make_client_manager(clients: Vec<Client>) -> ClientManager {
        let mut mgr = ClientManager::new();
        for c in clients {
            mgr.insert(c.win, c);
        }
        mgr
    }

    #[test]
    fn mode_change_to_floating_returns_plan() {
        let win = WindowId(1);
        let tag = TagMask::single(1).unwrap();
        let mut clients = make_client_manager(vec![make_client(win, tag, ClientMode::Tiling)]);

        let plan = update_window_mode(&mut clients, win, BaseClientMode::Floating);
        assert!(plan.is_some());
        let p = plan.unwrap();
        assert_eq!(
            p.change,
            WindowModeChange::ChangedToFloating {
                restored_geometry: true
            }
        );
        assert_eq!(p.border_width, 4); // restored from old_border_width
        assert_eq!(
            p.restore_geometry,
            Some(Rect {
                x: 10,
                y: 10,
                w: 200,
                h: 200
            })
        );

        // Check model state
        let client = clients.get(&win).unwrap();
        assert_eq!(client.mode, ClientMode::Floating);
        assert_eq!(client.border_width, 4);
    }

    #[test]
    fn mode_change_to_tiling_returns_plan() {
        let win = WindowId(1);
        let tag = TagMask::single(1).unwrap();
        let mut clients = make_client_manager(vec![make_client(win, tag, ClientMode::Floating)]);

        let plan = update_window_mode(&mut clients, win, BaseClientMode::Tiling);
        let p = plan.unwrap();
        assert_eq!(p.change, WindowModeChange::ChangedToTiling);
        assert_eq!(p.border_width, 2);

        // Check model state
        let client = clients.get(&win).unwrap();
        assert_eq!(client.mode, ClientMode::Tiling);
        assert_eq!(
            client.float_geo,
            Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 100
            }
        ); // saved from geo
    }

    #[test]
    fn mode_change_missing_client_returns_none() {
        let mut clients = ClientManager::new();
        let plan = update_window_mode(&mut clients, WindowId(99), BaseClientMode::Floating);
        assert!(plan.is_none());
    }

    #[test]
    fn mode_change_to_floating_from_floating_is_idempotent() {
        let win = WindowId(1);
        let tag = TagMask::single(1).unwrap();
        let mut client = make_client(win, tag, ClientMode::Floating);
        client.float_geo = Rect {
            x: 20,
            y: 20,
            w: 300,
            h: 300,
        };
        let mut clients = make_client_manager(vec![client]);

        let plan = update_window_mode(&mut clients, win, BaseClientMode::Floating);
        assert!(plan.is_some());
        let p = plan.unwrap();
        assert_eq!(
            p.change,
            WindowModeChange::ChangedToFloating {
                restored_geometry: true
            }
        );

        let client = clients.get(&win).unwrap();
        assert_eq!(client.mode, ClientMode::Floating);
    }

    #[test]
    fn mode_change_to_floating_with_no_saved_float_geo() {
        let win = WindowId(1);
        let tag = TagMask::single(1).unwrap();
        let mut client = make_client(win, tag, ClientMode::Tiling);
        // float_geo is zero (default) — should fall back to current geo
        client.float_geo = Rect::default();
        let mut clients = make_client_manager(vec![client]);

        let plan = update_window_mode(&mut clients, win, BaseClientMode::Floating);
        assert!(plan.is_some());
        let p = plan.unwrap();
        assert_eq!(
            p.change,
            WindowModeChange::ChangedToFloating {
                restored_geometry: true
            }
        );
        assert_eq!(
            p.restore_geometry,
            Some(Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 100
            })
        );
    }

    #[test]
    fn mode_change_to_tiling_defers_border_policy_to_layout() {
        let win = WindowId(1);
        let tag = TagMask::single(1).unwrap();
        let mut client = make_client(win, tag, ClientMode::Floating);
        client.snap_status = SnapPosition::None;
        client.border_width = 2;
        client.old_border_width = 2;
        let mut clients = make_client_manager(vec![client]);

        let plan = update_window_mode(&mut clients, win, BaseClientMode::Tiling);
        assert_eq!(plan.unwrap().change, WindowModeChange::ChangedToTiling);
        // Mode transitions do not guess layout policy from the global client
        // count; the arrange plan decides this from visible tiled clients.
        let client = clients.get(&win).unwrap();
        assert_eq!(client.border_width, 2);
    }
}
