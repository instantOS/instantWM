use crate::ipc_types::{ModeCommand, ModeInfo, Response};
use crate::wm::Wm;

fn apply_mode_change(wm: &mut Wm, next_mode: String) {
    let previous_mode = wm.g.behavior.current_mode.clone();
    if previous_mode == next_mode {
        return;
    }

    wm.g.behavior.current_mode = next_mode.clone();
    {
        let mut ctx = wm.ctx();
        crate::overview::handle_mode_transition(&mut ctx, &previous_mode, &next_mode);
    }
}

pub fn handle_mode_command(wm: &mut Wm, cmd: ModeCommand) -> Response {
    match cmd {
        ModeCommand::List => {
            let modes = &wm.g.cfg.bindings.modes;
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
            if !wm.g.cfg.bindings.modes.contains_key(&name)
                && name != "default"
                && name != crate::overview::OVERVIEW_MODE_NAME
            {
                return Response::err(format!("Mode '{}' not found", name));
            }
            apply_mode_change(wm, name.clone());

            wm.ctx().grab_keys();

            wm.bar.mark_dirty();
            Response::Message(format!("Switched to mode '{}'", name))
        }
        ModeCommand::Toggle(name) => {
            if !wm.g.cfg.bindings.modes.contains_key(&name)
                && name != "default"
                && name != crate::overview::OVERVIEW_MODE_NAME
            {
                return Response::err(format!("Mode '{}' not found", name));
            }

            let new_mode = if wm.g.behavior.current_mode == name {
                "default".to_string()
            } else {
                name
            };

            apply_mode_change(wm, new_mode.clone());

            wm.ctx().grab_keys();

            wm.bar.mark_dirty();
            Response::Message(format!("Toggled mode, now in '{}'", new_mode))
        }
    }
}
