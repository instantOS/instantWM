use crate::core_state::ActiveWmMode;
use crate::ipc_types::{ModeCommand, ModeInfo, Response};
use crate::wm::Wm;

fn apply_mode_change(wm: &mut Wm, next_mode: ActiveWmMode) {
    let mut ctx = wm.ctx();
    ctx.set_current_mode(next_mode);
}

pub fn handle_mode_command(wm: &mut Wm, cmd: ModeCommand) -> Response {
    match cmd {
        ModeCommand::List => {
            let modes = &wm.core.config.bindings.modes;
            let current_mode = &wm.core.behavior.current_mode;

            if modes.is_empty() {
                return Response::ModeList(Vec::new());
            }

            let mode_list: Vec<ModeInfo> = modes
                .iter()
                .map(|(name, mode)| ModeInfo {
                    name: name.clone(),
                    description: mode.description.clone(),
                    is_active: current_mode.as_str() == name,
                })
                .collect();

            Response::ModeList(mode_list)
        }
        ModeCommand::Set(name) => {
            if name == crate::core_state::TREE_PLACEMENT_MODE_NAME {
                return Response::err(
                    "Mode 'placement' can only be entered by begin_tree_placement".to_string(),
                );
            }
            if !wm.core.config.bindings.modes.contains_key(&name)
                && !matches!(
                    ActiveWmMode::from_name(&name),
                    ActiveWmMode::Default | ActiveWmMode::Overview
                )
            {
                return Response::err(format!("Mode '{}' not found", name));
            }
            apply_mode_change(wm, ActiveWmMode::from_name(&name));

            Response::Message(format!("Switched to mode '{}'", name))
        }
        ModeCommand::Toggle(name) => {
            if name == crate::core_state::TREE_PLACEMENT_MODE_NAME
                && wm.core.behavior.current_mode.as_str() != name
            {
                return Response::err(
                    "Mode 'placement' can only be entered by begin_tree_placement".to_string(),
                );
            }
            if !wm.core.config.bindings.modes.contains_key(&name)
                && !matches!(
                    ActiveWmMode::from_name(&name),
                    ActiveWmMode::Default | ActiveWmMode::Overview
                )
            {
                return Response::err(format!("Mode '{}' not found", name));
            }

            let new_mode = if wm.core.behavior.current_mode.as_str() == name {
                ActiveWmMode::Default
            } else {
                ActiveWmMode::from_name(&name)
            };

            let mode_name = new_mode.as_str().to_string();
            apply_mode_change(wm, new_mode);

            Response::Message(format!("Toggled mode, now in '{}'", mode_name))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::config::ModeConfig;

    #[test]
    fn ipc_cannot_enter_interaction_owned_placement_mode() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.config.bindings.modes.insert(
            crate::core_state::TREE_PLACEMENT_MODE_NAME.to_string(),
            ModeConfig::default(),
        );

        let response = handle_mode_command(
            &mut wm,
            ModeCommand::Set(crate::core_state::TREE_PLACEMENT_MODE_NAME.to_string()),
        );

        assert!(
            matches!(response, Response::Err(message) if message.contains("begin_tree_placement"))
        );
        assert_eq!(wm.core.behavior.current_mode, ActiveWmMode::Default);
    }
}
