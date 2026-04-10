use crate::ipc_types::{Response, ToggleCommand};
use crate::toggles::{
    toggle_alt_tag, toggle_animated, toggle_focus_follows_float_mouse, toggle_focus_follows_mouse,
    toggle_show_tags,
};
use crate::types::ToggleAction;
use crate::wm::Wm;

pub fn handle_toggle_command(wm: &mut Wm, cmd: ToggleCommand) -> Response {
    let mut ctx = wm.ctx();
    match cmd {
        ToggleCommand::Animated(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            ctx.with_behavior_mut(|behavior| toggle_animated(behavior, action));
        }
        ToggleCommand::FocusFollowsMouse(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            ctx.with_behavior_mut(|behavior| toggle_focus_follows_mouse(behavior, action));
        }
        ToggleCommand::FocusFollowsFloatMouse(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            ctx.with_behavior_mut(|behavior| toggle_focus_follows_float_mouse(behavior, action));
        }
        ToggleCommand::AltTag(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_alt_tag(&mut ctx, action);
        }
        ToggleCommand::HideTags(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_show_tags(&mut ctx, action);
        }
    }
    Response::ok()
}
