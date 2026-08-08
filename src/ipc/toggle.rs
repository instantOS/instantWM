use crate::ipc_types::{Response, ToggleCommand};
use crate::toggles::{set_bottom_bar_shown, toggle_alt_tag, toggle_hide_tags};
use crate::wm::Wm;

pub fn handle_toggle_command(wm: &mut Wm, cmd: ToggleCommand) -> Response {
    let mut ctx = wm.ctx();
    match cmd {
        ToggleCommand::Animated(action) => {
            ctx.with_behavior_mut(|behavior| behavior.toggle_animated(action));
        }
        ToggleCommand::FocusFollowsMouse(mode) => {
            ctx.with_behavior_mut(|behavior| behavior.set_focus_follows_mouse(mode));
        }
        ToggleCommand::FocusFollowsFloatMouse(action) => {
            ctx.with_behavior_mut(|behavior| behavior.toggle_focus_follows_float_mouse(action));
        }
        ToggleCommand::AltTag(action) => {
            toggle_alt_tag(&mut ctx, action);
        }
        ToggleCommand::HideTags(action) => {
            toggle_hide_tags(&mut ctx, action);
        }
        ToggleCommand::BottomBar(action) => {
            let mut shown = ctx
                .core()
                .model()
                .expect_selected_monitor()
                .shows_bottom_bar();
            action.apply(&mut shown);
            set_bottom_bar_shown(&mut ctx, shown);
        }
    }
    Response::ok()
}
