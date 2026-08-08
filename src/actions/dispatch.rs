use crate::actions::{ButtonAction, KeyAction};
use crate::client::{close_win, kill_client};
use crate::contexts::WmCtx;
use crate::floating::{
    DEFAULT_EDGE_SCRATCHPAD_NAME, scratchpad_hide_name, scratchpad_show_name, toggle_floating,
};
use crate::model::WmModel;
use crate::mouse::{resize_aspect_mouse, window_title_mouse_handler};
use crate::toggles::toggle_locked;
use crate::types::TagMask;

use super::named::execute_named_action;

fn button_target_client(
    model: &WmModel,
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
        .or_else(|| model.selected_win())
}

pub fn execute_key_action(ctx: &mut WmCtx<'_>, action: &KeyAction) {
    crate::overview::prepare_key_action(ctx, action);
    match action {
        KeyAction::Sequence(actions) => {
            for action in actions {
                execute_key_action(ctx, action);
            }
        }
        KeyAction::Named { action, args } => execute_named_action(ctx, *action, args),
        KeyAction::ViewTag { tag_idx } => {
            if let Some(mask) = TagMask::from_index(*tag_idx) {
                crate::tags::view::view_tags(ctx, mask);
            }
        }
        KeyAction::ToggleViewTag { tag_idx } => {
            if let Some(mask) = TagMask::from_index(*tag_idx) {
                crate::tags::view::toggle_view(ctx, mask);
            }
        }
        KeyAction::SetClientTag { tag_idx } => {
            if let Some(win) = ctx.core().model().selected_win()
                && let Some(mask) = TagMask::from_index(*tag_idx)
            {
                crate::tags::client_tags::set_client_tag(ctx, win, mask);
            }
        }
        KeyAction::FollowClientTag { tag_idx } => {
            if let Some(win) = ctx.core().model().selected_win()
                && let Some(mask) = TagMask::from_index(*tag_idx)
            {
                crate::tags::client_tags::follow_tag(ctx, win, mask);
            }
        }
        KeyAction::ToggleClientTag { tag_idx } => {
            if let Some(win) = ctx.core().model().selected_win()
                && let Some(mask) = TagMask::from_index(*tag_idx)
            {
                crate::tags::client_tags::toggle_tag(ctx, win, mask);
            }
        }
        KeyAction::SwapTags { tag_idx } => {
            if let Some(mask) = TagMask::from_index(*tag_idx) {
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
    crate::overview::prepare_button_action(ctx, action);
    match action {
        ButtonAction::Named { action, args } => execute_named_action(ctx, *action, args),
        ButtonAction::WindowTitleMouseHandler => {
            let Some(crate::types::BarPosition::WinTitle(win)) = arg.bar_position() else {
                return;
            };
            window_title_mouse_handler(ctx, win, arg.btn, arg.source, arg.root);
        }
        ButtonAction::CloseClickedTitleWindow => {
            let Some(crate::types::BarPosition::WinTitle(win)) = arg.bar_position() else {
                return;
            };
            close_win(ctx, win);
        }
        ButtonAction::DragTagBegin => {
            if let Some(pos) = arg.bar_position() {
                let _ = crate::mouse::drag::drag_tag_begin(ctx, pos, arg.btn, arg.source, arg.root);
            }
        }
        ButtonAction::ToggleClickedViewTag => {
            if let Some(crate::types::BarPosition::Tag(idx)) = arg.bar_position() {
                crate::tags::view::toggle_view_tag(ctx, idx);
            }
        }
        ButtonAction::SetSelectedClientClickedTag => {
            if let Some(win) = ctx.core().model().selected_win()
                && let Some(mask) = arg.bar_position().and_then(|pos| pos.to_tag_mask())
            {
                crate::tags::client_tags::set_client_tag(ctx, win, mask);
            }
        }
        ButtonAction::ToggleSelectedClientClickedTag => {
            if let Some(win) = ctx.core().model().selected_win()
                && let Some(mask) = arg.bar_position().and_then(|pos| pos.to_tag_mask())
            {
                crate::tags::client_tags::toggle_tag(ctx, win, mask);
            }
        }
        ButtonAction::ClientMoveDrag => {
            if let Some(win) = button_target_client(ctx.core().model(), &arg) {
                crate::focus::focus(ctx, Some(win));
                crate::mouse::drag::thresholded_client_drag(
                    ctx, win, arg.btn, arg.source, arg.root, true,
                );
            }
        }
        ButtonAction::ResizeSelectedAspect => {
            if let Some(win) = button_target_client(ctx.core().model(), &arg) {
                crate::focus::focus(ctx, Some(win));
                resize_aspect_mouse(ctx, win, arg.btn, arg.source);
            }
        }
        ButtonAction::KillSelectedClient => {
            if let Some(win) = button_target_client(ctx.core().model(), &arg) {
                kill_client(ctx, win);
            }
        }
        ButtonAction::ToggleLockSelectedClient => {
            if let Some(win) = button_target_client(ctx.core().model(), &arg) {
                toggle_locked(ctx, win);
            }
        }
        ButtonAction::ReorderSelected { direction } => {
            if !matches!(
                crate::layouts::reorder_maximized_stack(ctx, *direction),
                crate::layouts::MaximizedStackReorder::NotApplicable
            ) {
                return;
            }
            if let Some(win) = ctx.core().model().selected_win()
                && ctx
                    .core_mut()
                    .model_mut()
                    .move_client_in_stack(win, *direction)
            {
                crate::focus::focus(ctx, Some(win));
                let monitor_id = ctx.core().model().selected_monitor_id();
                ctx.core_mut().queue_layout_for_monitor_urgent(monitor_id);
            }
        }
        ButtonAction::ScaleSelected { percent } => {
            if let Some(win) = button_target_client(ctx.core().model(), &arg) {
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
        ButtonAction::ResizeMouseFromCursor => {
            if let Some(win) = button_target_client(ctx.core().model(), &arg) {
                crate::focus::select_monitor_for_client(ctx, win);
                crate::focus::focus(ctx, Some(win));
                crate::mouse::drag::thresholded_client_drag(
                    ctx, win, arg.btn, arg.source, arg.root, true,
                );
            }
        }
        ButtonAction::BottomBarDrag { left, right, up } => {
            let Some(monitor_id) =
                crate::mouse::pointer::bottom_bar_monitor_at(ctx.core().model(), arg.root)
            else {
                return;
            };
            crate::mouse::drag::bottom_bar_gesture_begin(
                ctx,
                arg.btn,
                arg.source,
                monitor_id,
                arg.root,
                left.clone(),
                right.clone(),
                up.clone(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::NamedAction;
    use crate::backend::{Backend, wayland::WaylandBackend};
    use crate::core_state::ActiveWmMode;
    use crate::wm::Wm;

    #[test]
    fn key_action_sequences_execute_every_action_in_order() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let action = KeyAction::Sequence(vec![
            KeyAction::Named {
                action: NamedAction::SetMode,
                args: vec!["first".to_string()],
            },
            KeyAction::Sequence(vec![
                KeyAction::Named {
                    action: NamedAction::SetMode,
                    args: vec!["second".to_string()],
                },
                KeyAction::Named {
                    action: NamedAction::SetMode,
                    args: vec!["final".to_string()],
                },
            ]),
        ]);

        execute_key_action(&mut wm.ctx(), &action);

        assert_eq!(
            wm.core.behavior.current_mode,
            ActiveWmMode::Named("final".to_string())
        );
    }
}
