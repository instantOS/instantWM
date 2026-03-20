use crate::ipc_types::{IpcResponse, ToggleCommand};
use crate::toggles::{
    toggle_alt_tag, toggle_animated, toggle_desktop_mode, toggle_focus_follows_float_mouse,
    toggle_focus_follows_mouse, toggle_show_tags,
};
use crate::types::ToggleAction;
use crate::wm::Wm;

pub fn handle_toggle_command(wm: &mut Wm, cmd: ToggleCommand) -> IpcResponse {
    let mut ctx = wm.ctx();
    match cmd {
        ToggleCommand::Animated(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_animated(&mut ctx.core_mut().globals_mut().behavior, action);
        }
        ToggleCommand::FocusFollowsMouse(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_focus_follows_mouse(&mut ctx.core_mut().globals_mut().behavior, action);
        }
        ToggleCommand::FocusFollowsFloatMouse(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_focus_follows_float_mouse(&mut ctx.core_mut().globals_mut().behavior, action);
        }
        ToggleCommand::DesktopMode(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_desktop_mode(&mut ctx, action);
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
