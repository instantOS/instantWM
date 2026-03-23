use crate::backend::x11::grab::grab_keys_x11;
use crate::contexts::WmCtx;
use crate::ipc_types::{ModeCommand, ModeInfo, Response};
use crate::wm::Wm;

pub fn handle_mode_command(wm: &mut Wm, cmd: ModeCommand) -> Response {
    match cmd {
        ModeCommand::List => {
            let modes = &wm.g.cfg.modes;
            let current_mode = &wm.g.behavior.current_mode;

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
            if !wm.g.cfg.modes.contains_key(&name) && name != "default" {
                return Response::err(format!("Mode '{}' not found", name));
            }
            wm.g.behavior.current_mode = name.clone();

            if let WmCtx::X11(x11) = wm.ctx() {
                grab_keys_x11(&x11.core, &x11.x11, x11.x11_runtime);
            }

            wm.bar.mark_dirty();
            Response::Message(format!("Switched to mode '{}'", name))
        }
        ModeCommand::Toggle(name) => {
            if !wm.g.cfg.modes.contains_key(&name) && name != "default" {
                return Response::err(format!("Mode '{}' not found", name));
            }

            let new_mode = if wm.g.behavior.current_mode == name {
                "default".to_string()
            } else {
                name
            };

            wm.g.behavior.current_mode = new_mode.clone();

            if let WmCtx::X11(x11) = wm.ctx() {
                grab_keys_x11(&x11.core, &x11.x11, x11.x11_runtime);
            }

            wm.bar.mark_dirty();
            Response::Message(format!("Toggled mode, now in '{}'", new_mode))
        }
    }
}
