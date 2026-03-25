use crate::actions::{ButtonAction, KeyAction};
use crate::client::{close_win, kill_client};
use crate::contexts::{WmCtx, WmCtxX11};
use crate::floating::{hide_overlay, show_overlay, toggle_floating};
use crate::monitor::{Direction as PushDirection, reorder_client};
use crate::mouse::{
    drag_tag, gesture_mouse, resize_aspect_mouse, resize_mouse_from_cursor,
    window_title_mouse_handler,
};
use crate::toggles::toggle_locked;
use crate::types::TagMask;

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

pub fn execute_key_action(ctx: &mut WmCtx<'_>, action: &KeyAction) {
    match action {
        KeyAction::Named { action, args } => execute_named_action(ctx, *action, args),
        KeyAction::ViewTag { tag_idx } => {
            if let Some(mask) = tag_mask_from_idx(*tag_idx) {
                crate::tags::view::view(ctx, mask);
            }
        }
        KeyAction::ToggleViewTag { tag_idx } => {
            if let Some(mask) = tag_mask_from_idx(*tag_idx) {
                crate::tags::view::toggle_view_ctx(ctx, mask);
            }
        }
        KeyAction::SetClientTag { tag_idx } => {
            if let Some(win) = ctx.selected_client()
                && let Some(mask) = tag_mask_from_idx(*tag_idx)
            {
                crate::tags::client_tags::set_client_tag_ctx(ctx, win, mask);
            }
        }
        KeyAction::FollowClientTag { tag_idx } => {
            if let Some(win) = ctx.selected_client()
                && let Some(mask) = tag_mask_from_idx(*tag_idx)
            {
                crate::tags::client_tags::follow_tag_ctx(ctx, win, mask);
            }
        }
        KeyAction::ToggleClientTag { tag_idx } => {
            if let Some(win) = ctx.selected_client()
                && let Some(mask) = tag_mask_from_idx(*tag_idx)
            {
                crate::tags::client_tags::toggle_tag_ctx(ctx, win, mask);
            }
        }
        KeyAction::SwapTags { tag_idx } => {
            if let Some(mask) = tag_mask_from_idx(*tag_idx) {
                crate::tags::view::swap_tags_ctx(ctx, mask);
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
            let crate::types::BarPosition::WinTitle(win) = arg.pos else {
                return;
            };
            window_title_mouse_handler(ctx, win, arg.btn, arg.rx, arg.ry);
        }
        ButtonAction::CloseClickedTitleWindow => {
            let crate::types::BarPosition::WinTitle(win) = arg.pos else {
                return;
            };
            close_win(ctx, win);
        }
        ButtonAction::DragTagBegin => match ctx {
            WmCtx::X11(ctx_x11) => drag_tag(ctx_x11, arg.pos, arg.btn, arg.rx),
            WmCtx::Wayland(_) => {
                let _ = crate::mouse::drag::drag_tag_begin(ctx, arg.pos, arg.btn);
            }
        },
        ButtonAction::ToggleClickedViewTag => {
            if let crate::types::BarPosition::Tag(idx) = arg.pos {
                crate::tags::view::toggle_view_tag(ctx, idx);
            }
        }
        ButtonAction::SetSelectedClientClickedTag => {
            if let Some(win) = ctx.selected_client()
                && let Some(mask) = tag_mask_from_pos(arg.pos)
            {
                crate::tags::client_tags::set_client_tag_ctx(ctx, win, mask);
            }
        }
        ButtonAction::ToggleSelectedClientClickedTag => {
            if let Some(win) = ctx.selected_client()
                && let Some(mask) = tag_mask_from_pos(arg.pos)
            {
                crate::tags::client_tags::toggle_tag_ctx(ctx, win, mask);
            }
        }
        ButtonAction::FollowSelectedClientClickedTag => {
            if let Some(win) = ctx.selected_client()
                && let Some(mask) = tag_mask_from_pos(arg.pos)
            {
                crate::tags::client_tags::follow_tag_ctx(ctx, win, mask);
            }
        }
        ButtonAction::ClientMoveDrag => match ctx {
            WmCtx::X11(ctx_x11) => {
                crate::backend::x11::mouse::move_mouse_x11(ctx_x11, arg.btn, None)
            }
            WmCtx::Wayland(_) => {
                if let Some(win) = ctx.selected_client() {
                    crate::mouse::drag::title_drag_begin(ctx, win, arg.btn, arg.rx, arg.ry, false);
                }
            }
        },
        ButtonAction::ResizeSelectedAspect => {
            if let Some(win) = ctx.selected_client() {
                resize_aspect_mouse(ctx, win, arg.btn);
            }
        }
        ButtonAction::KillSelectedClient => {
            if let Some(win) = ctx.selected_client() {
                kill_client(ctx, win);
            }
        }
        ButtonAction::ToggleLockSelectedClient => {
            if let Some(win) = ctx.selected_client() {
                toggle_locked(ctx, win);
            }
        }
        ButtonAction::GestureMouse => gesture_mouse(ctx, arg.btn),
        ButtonAction::ReorderSelected { up } => {
            if let Some(win) = ctx.selected_client() {
                reorder_client(
                    ctx,
                    win,
                    if *up {
                        PushDirection::Up
                    } else {
                        PushDirection::Down
                    },
                );
            }
        }
        ButtonAction::ScaleSelected { percent } => {
            if let Some(win) = ctx.selected_client() {
                crate::client::geometry::scale_client(ctx, win, *percent);
            }
        }
        ButtonAction::HideOverlay => hide_overlay(ctx),
        ButtonAction::ShowOverlay => show_overlay(ctx),
        ButtonAction::ToggleFloatingSelected => toggle_floating(ctx),
        ButtonAction::ResizeMouseFromCursor => resize_mouse_from_cursor(ctx, arg.btn),
    }
}

pub fn execute_button_action_x11(
    ctx: &mut WmCtxX11<'_>,
    action: &ButtonAction,
    arg: crate::types::ButtonArg,
) {
    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    execute_button_action(&mut wm_ctx, action, arg);
}
