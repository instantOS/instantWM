use crate::actions::{ButtonAction, KeyAction};
use crate::client::{close_win, kill_client};
use crate::contexts::{CoreCtx, WmCtx};
use crate::floating::{
    DEFAULT_EDGE_SCRATCHPAD_NAME, scratchpad_hide_name, scratchpad_show_name, toggle_floating,
};
use crate::monitor::reorder_client;
use crate::mouse::{
    drag_tag, resize_aspect_mouse, resize_mouse_from_cursor, sidebar_gesture_begin,
    window_title_mouse_handler,
};
use crate::toggles::toggle_locked;
use crate::types::TagMask;
use crate::types::VerticalDirection;

use super::named::execute_named_action;

fn tag_mask_from_idx(tag_idx: usize) -> Option<TagMask> {
    TagMask::single(tag_idx + 1)
}

fn tag_mask_from_pos(pos: crate::types::BarPosition) -> Option<TagMask> {
    match pos {
        crate::types::BarPosition::Tag(idx) => tag_mask_from_idx(idx),
        _ => None,
    }
}

fn button_target_client(
    core: &CoreCtx<'_>,
    arg: &crate::types::ButtonArg,
) -> Option<crate::types::WindowId> {
    arg.window
        .or(match arg.target {
            crate::types::ButtonTarget::Bar(crate::types::BarPosition::WinTitle(win))
            | crate::types::ButtonTarget::Bar(crate::types::BarPosition::CloseButton(win))
            | crate::types::ButtonTarget::Bar(crate::types::BarPosition::ResizeWidget(win)) => {
                Some(win)
            }
            _ => None,
        })
        .or_else(|| core.selected_client())
}

pub fn execute_key_action(ctx: &mut WmCtx<'_>, action: &KeyAction) {
    match action {
        KeyAction::Named { action, args } => execute_named_action(ctx, *action, args),
        KeyAction::ViewTag { tag_idx } => {
            if let Some(mask) = tag_mask_from_idx(*tag_idx) {
                crate::tags::view::view_tags(ctx, mask);
            }
        }
        KeyAction::ToggleViewTag { tag_idx } => {
            if let Some(mask) = tag_mask_from_idx(*tag_idx) {
                crate::tags::view::toggle_view(ctx, mask);
            }
        }
        KeyAction::SetClientTag { tag_idx } => {
            if let Some(win) = ctx.core().selected_client()
                && let Some(mask) = tag_mask_from_idx(*tag_idx)
            {
                crate::tags::client_tags::set_client_tag(ctx, win, mask);
            }
        }
        KeyAction::FollowClientTag { tag_idx } => {
            if let Some(win) = ctx.core().selected_client()
                && let Some(mask) = tag_mask_from_idx(*tag_idx)
            {
                crate::tags::client_tags::follow_tag(ctx, win, mask);
            }
        }
        KeyAction::ToggleClientTag { tag_idx } => {
            if let Some(win) = ctx.core().selected_client()
                && let Some(mask) = tag_mask_from_idx(*tag_idx)
            {
                crate::tags::client_tags::toggle_tag(ctx, win, mask);
            }
        }
        KeyAction::SwapTags { tag_idx } => {
            if let Some(mask) = tag_mask_from_idx(*tag_idx) {
                crate::tags::view::swap_tags(ctx, mask);
            }
        }
    }
}

pub fn execute_button_action(
    ctx: &mut WmCtx<'_>,
    action: &ButtonAction,
    arg: crate::types::ButtonArg,
) {
    match action {
        ButtonAction::Named { action, args } => execute_named_action(ctx, *action, args),
        ButtonAction::WindowTitleMouseHandler => {
            let Some(crate::types::BarPosition::WinTitle(win)) = arg.bar_position() else {
                return;
            };
            window_title_mouse_handler(ctx, win, arg.btn, arg.root);
        }
        ButtonAction::CloseClickedTitleWindow => {
            let Some(crate::types::BarPosition::WinTitle(win)) = arg.bar_position() else {
                return;
            };
            close_win(ctx, win);
        }
        ButtonAction::DragTagBegin => match ctx {
            WmCtx::X11(ctx_x11) => {
                if let Some(pos) = arg.bar_position() {
                    drag_tag(ctx_x11, pos, arg.btn, arg.root.x);
                }
            }
            WmCtx::Wayland(_) => {
                if let Some(pos) = arg.bar_position() {
                    let _ = crate::mouse::drag::drag_tag_begin(ctx, pos, arg.btn);
                }
            }
        },
        ButtonAction::ToggleClickedViewTag => {
            if let Some(crate::types::BarPosition::Tag(idx)) = arg.bar_position() {
                crate::tags::view::toggle_view_tag(ctx, idx);
            }
        }
        ButtonAction::SetSelectedClientClickedTag => {
            if let Some(win) = ctx.core().selected_client()
                && let Some(pos) = arg.bar_position()
                && let Some(mask) = tag_mask_from_pos(pos)
            {
                crate::tags::client_tags::set_client_tag(ctx, win, mask);
            }
        }
        ButtonAction::ToggleSelectedClientClickedTag => {
            if let Some(win) = ctx.core().selected_client()
                && let Some(pos) = arg.bar_position()
                && let Some(mask) = tag_mask_from_pos(pos)
            {
                crate::tags::client_tags::toggle_tag(ctx, win, mask);
            }
        }
        ButtonAction::FollowSelectedClientClickedTag => {
            if let Some(win) = ctx.core().selected_client()
                && let Some(pos) = arg.bar_position()
                && let Some(mask) = tag_mask_from_pos(pos)
            {
                crate::tags::client_tags::follow_tag(ctx, win, mask);
            }
        }
        ButtonAction::ClientMoveDrag => match ctx {
            WmCtx::X11(ctx_x11) => {
                if let Some(win) = button_target_client(&ctx_x11.core, &arg) {
                    let mut wm_ctx = WmCtx::X11(ctx_x11.reborrow());
                    crate::focus::focus(&mut wm_ctx, Some(win));
                }
                crate::backend::x11::mouse::move_mouse_x11(ctx_x11, arg.btn, None)
            }
            WmCtx::Wayland(_) => {
                if let Some(win) = button_target_client(ctx.core(), &arg) {
                    crate::focus::focus(ctx, Some(win));
                    crate::mouse::drag::title_drag_begin(ctx, win, arg.btn, arg.root, false);
                }
            }
        },
        ButtonAction::ResizeSelectedAspect => {
            if let Some(win) = button_target_client(ctx.core(), &arg) {
                crate::focus::focus(ctx, Some(win));
                resize_aspect_mouse(ctx, win, arg.btn);
            }
        }
        ButtonAction::KillSelectedClient => {
            if let Some(win) = button_target_client(ctx.core(), &arg) {
                kill_client(ctx, win);
            }
        }
        ButtonAction::ToggleLockSelectedClient => {
            if let Some(win) = button_target_client(ctx.core(), &arg) {
                toggle_locked(ctx, win);
            }
        }
        ButtonAction::SidebarGestureBegin => sidebar_gesture_begin(ctx, arg.btn),
        ButtonAction::ReorderSelected { up } => {
            if let Some(win) = ctx.core().selected_client() {
                reorder_client(
                    ctx,
                    win,
                    if *up {
                        VerticalDirection::Up
                    } else {
                        VerticalDirection::Down
                    },
                );
            }
        }
        ButtonAction::ScaleSelected { percent } => {
            if let Some(win) = button_target_client(ctx.core(), &arg) {
                crate::client::geometry::scale_client(ctx, win, *percent);
            }
        }
        ButtonAction::HideEdgeScratchpad => {
            scratchpad_hide_name(ctx, DEFAULT_EDGE_SCRATCHPAD_NAME);
        }
        ButtonAction::ShowEdgeScratchpad => {
            let _ = scratchpad_show_name(ctx, DEFAULT_EDGE_SCRATCHPAD_NAME);
        }
        ButtonAction::ToggleFloatingSelected => toggle_floating(ctx),
        ButtonAction::ResizeMouseFromCursor => resize_mouse_from_cursor(ctx, arg.btn),
    }
}
