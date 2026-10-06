use crate::contexts::WmCtx;
use crate::ipc_types::{ModeInfo, Response};

pub fn list_modes(ctx: &mut WmCtx<'_>) -> Response {
    let modes = &ctx.config().bindings.modes;
    let current_mode = &ctx.state().behavior.current_mode;

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
