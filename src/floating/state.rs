//! Floating state transitions and geometry persistence.

use crate::constants::animation::DEFAULT_FRAME_COUNT;
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::layouts::arrange;
use crate::types::*;

pub fn restore_floating_geometry(ctx: &mut WmCtx, win: WindowId) {
    if let Some(rect) = ctx
        .core()
        .model()
        .client(win)
        .map(Client::effective_float_geo)
    {
        ctx.move_resize(win, rect, MoveResizeOptions::hinted_immediate(false));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum WindowModeChange {
    MissingClient,
    ChangedToFloating { restored_geometry: Rect },
    ChangedToTiling,
}

impl WindowModeChange {
    pub fn restored_geometry(self) -> Option<Rect> {
        match self {
            WindowModeChange::ChangedToFloating { restored_geometry } => Some(restored_geometry),
            WindowModeChange::MissingClient | WindowModeChange::ChangedToTiling => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WindowModePlan {
    pub(crate) change: WindowModeChange,
    pub(crate) border_width: i32,
}

pub(crate) fn update_window_mode(client: &mut Client, mode: BaseClientMode) -> WindowModePlan {
    match mode {
        BaseClientMode::Floating => {
            client.replace_mode_with_base(BaseClientMode::Floating);
            client.restore_border_width();
            let border_width = client.border_width;
            let restored_geometry = client.effective_float_geo();
            WindowModePlan {
                change: WindowModeChange::ChangedToFloating { restored_geometry },
                border_width,
            }
        }
        BaseClientMode::Tiling => {
            client.save_floating_geometry();
            client.replace_mode_with_base(BaseClientMode::Tiling);
            WindowModePlan {
                change: WindowModeChange::ChangedToTiling,
                border_width: client.border_width,
            }
        }
    }
}

/// Set a window to floating or tiled mode.
///
/// Handles border updates and geometry changes but not caller-owned animation.
pub fn set_window_mode(ctx: &mut WmCtx, win: WindowId, mode: BaseClientMode) -> WindowModeChange {
    let Some(client) = ctx.core_mut().model_mut().client_mut(win) else {
        return WindowModeChange::MissingClient;
    };
    let plan = update_window_mode(client, mode);

    match plan.change {
        WindowModeChange::ChangedToFloating { restored_geometry } => {
            if let WmCtx::X11(x11) = ctx {
                x11.x11.set_border_width(win, 0);
                x11.x11.set_border_width(win, plan.border_width);
                crate::backend::x11::floating::apply_floating_borderscheme(
                    &x11.x11,
                    win,
                    x11.x11_runtime,
                );
            }

            ctx.move_resize(
                win,
                restored_geometry,
                MoveResizeOptions::hinted_immediate(false),
            );
            plan.change
        }
        WindowModeChange::ChangedToTiling => plan.change,
        WindowModeChange::MissingClient => {
            unreachable!("an existing client produced a missing transition")
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

    let Some((is_floating, is_fixed)) = ctx
        .core()
        .state()
        .model
        .client(win)
        .map(|c| (c.mode().is_floating(), c.is_fixed_size))
    else {
        return;
    };
    let target_mode = if !is_floating || is_fixed {
        BaseClientMode::Floating
    } else {
        BaseClientMode::Tiling
    };
    let mode_change = set_window_mode(ctx, win, target_mode);

    // Animate when going to floating mode
    if let Some(saved_geo) = mode_change.restored_geometry() {
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
    use super::WindowModeChange;
    use super::update_window_mode;
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

    #[test]
    fn mode_change_to_floating_returns_plan() {
        let win = WindowId(1);
        let tag = TagMask::single(1).unwrap();
        let mut client = make_client(win, tag, ClientMode::Tiling);

        let p = update_window_mode(&mut client, BaseClientMode::Floating);
        assert_eq!(
            p.change,
            WindowModeChange::ChangedToFloating {
                restored_geometry: Rect {
                    x: 10,
                    y: 10,
                    w: 200,
                    h: 200
                }
            }
        );
        assert_eq!(p.border_width, 4); // restored from old_border_width

        // Check model state
        assert_eq!(client.mode(), ClientMode::Floating);
        assert_eq!(client.border_width, 4);
    }

    #[test]
    fn mode_change_to_tiling_returns_plan() {
        let win = WindowId(1);
        let tag = TagMask::single(1).unwrap();
        let mut client = make_client(win, tag, ClientMode::Floating);

        let p = update_window_mode(&mut client, BaseClientMode::Tiling);
        assert_eq!(p.change, WindowModeChange::ChangedToTiling);
        assert_eq!(p.border_width, 2);

        // Check model state
        assert_eq!(client.mode(), ClientMode::Tiling);
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
        let p = update_window_mode(&mut client, BaseClientMode::Floating);
        assert_eq!(
            p.change,
            WindowModeChange::ChangedToFloating {
                restored_geometry: Rect {
                    x: 20,
                    y: 20,
                    w: 300,
                    h: 300
                }
            }
        );

        assert_eq!(client.mode(), ClientMode::Floating);
    }

    #[test]
    fn mode_change_to_floating_with_no_saved_float_geo() {
        let win = WindowId(1);
        let tag = TagMask::single(1).unwrap();
        let mut client = make_client(win, tag, ClientMode::Tiling);
        // float_geo is zero (default) — should fall back to current geo
        client.float_geo = Rect::default();
        let p = update_window_mode(&mut client, BaseClientMode::Floating);
        assert_eq!(
            p.change,
            WindowModeChange::ChangedToFloating {
                restored_geometry: Rect {
                    x: 0,
                    y: 0,
                    w: 100,
                    h: 100
                }
            }
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
        let plan = update_window_mode(&mut client, BaseClientMode::Tiling);
        assert_eq!(plan.change, WindowModeChange::ChangedToTiling);
        // Mode transitions do not guess layout policy from the global client
        // count; the arrange plan decides this from visible tiled clients.
        assert_eq!(client.border_width, 2);
    }
}
