use crate::ipc_types::{IpcResponse, ToggleCommand};
use crate::toggles::{
    alt_tab_free, toggle_alt_tag, toggle_animated, toggle_focus_follows_float_mouse,
    toggle_focus_follows_mouse, toggle_show_tags,
};
use crate::types::ToggleAction;
use crate::wm::Wm;

pub fn handle_toggle_command(wm: &mut Wm, cmd: ToggleCommand) -> IpcResponse {
    let mut ctx = wm.ctx();
    match cmd {
        ToggleCommand::Animated(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_animated(ctx.core_mut(), action);
        }
        ToggleCommand::FocusFollowsMouse(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_focus_follows_mouse(ctx.core_mut(), action);
        }
        ToggleCommand::FocusFollowsFloatMouse(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_focus_follows_float_mouse(ctx.core_mut(), action);
        }
        ToggleCommand::AltTab(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            alt_tab_free(&mut ctx, action);
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
    IpcResponse::ok("")
}
