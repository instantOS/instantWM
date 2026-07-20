use crate::contexts::WmCtx;
use crate::ipc_types::{ModeCommand, ModeInfo, Response};
use crate::wm::Wm;

fn apply_mode_change(wm: &mut Wm, next_mode: String) {
    let mut ctx = wm.ctx();
    ctx.set_current_mode(next_mode);
}

pub fn handle_mode_command(wm: &mut Wm, cmd: ModeCommand) -> Response {
    match cmd {
        ModeCommand::List => {
            let modes = &wm.core.config.bindings.modes;
            let current_mode = wm.core.behavior.current_mode.as_str();

            if modes.is_empty() {
                return Response::ModeList(Vec::new());
            }

            let mode_list: Vec<ModeInfo> = modes
                .iter()
                .map(|(name, mode)| ModeInfo {
                    name: name.clone(),
                    description: mode.description.clone(),
                    is_active: name == current_mode,
                })
                .collect();

            Response::ModeList(mode_list)
        }
        ModeCommand::Set(name) => {
            if !wm.core.config.bindings.modes.contains_key(&name)
                && name != "default"
                && name != crate::overview::OVERVIEW_MODE_NAME
            {
                return Response::err(format!("Mode '{}' not found", name));
            }
            apply_mode_change(wm, name.clone());

            if let WmCtx::X11(x11) = &mut wm.ctx() {
                crate::backend::x11::keyboard::grab_keys(
                    x11.core.state(),
                    &x11.x11,
                    x11.x11_runtime,
                );
            }

            Response::Message(format!("Switched to mode '{}'", name))
        }
        ModeCommand::Toggle(name) => {
            if !wm.core.config.bindings.modes.contains_key(&name)
                && name != "default"
                && name != crate::overview::OVERVIEW_MODE_NAME
            {
                return Response::err(format!("Mode '{}' not found", name));
            }

            let new_mode = if wm.core.behavior.current_mode.as_str() == name {
                "default".to_string()
            } else {
                name
            };

            apply_mode_change(wm, new_mode.clone());

            if let WmCtx::X11(x11) = &mut wm.ctx() {
                crate::backend::x11::keyboard::grab_keys(
                    x11.core.state(),
                    &x11.x11,
                    x11.x11_runtime,
                );
            }

            Response::Message(format!("Toggled mode, now in '{}'", new_mode))
        }
    }
}
