use crate::ipc_types::{Response, ToggleCommand};
use crate::toggles::{toggle_alt_tag, toggle_show_tags};
use crate::wm::Wm;

pub fn handle_toggle_command(wm: &mut Wm, cmd: ToggleCommand) -> Response {
    let mut ctx = wm.ctx();
    match cmd {
        ToggleCommand::Animated(action) => {
            ctx.with_behavior_mut(|behavior| behavior.toggle_animated(action));
        }
        ToggleCommand::FocusFollowsMouse(action) => {
            ctx.with_behavior_mut(|behavior| behavior.toggle_focus_follows_mouse(action));
        }
        ToggleCommand::FocusFollowsFloatMouse(action) => {
            ctx.with_behavior_mut(|behavior| behavior.toggle_focus_follows_float_mouse(action));
        }
        ToggleCommand::AltTag(action) => {
            toggle_alt_tag(&mut ctx, action);
        }
        ToggleCommand::HideTags(action) => {
            toggle_show_tags(&mut ctx, action);
        }
    }
    Response::ok()
}
