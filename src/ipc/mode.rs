use crate::ipc_types::{ModeInfo, Response};

pub fn list_modes(state: &crate::core_state::CoreState) -> Response {
    let modes = &state.config.bindings.modes;
    let current_mode = &state.behavior.current_mode;

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
