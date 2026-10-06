use crate::ipc_types::Response;

pub fn set_wallpaper(ctx: &mut crate::contexts::WmCtx<'_>, path: String) -> Response {
    match ctx.set_wallpaper(&path) {
        Ok(()) => Response::Message(format!("Wallpaper set to {}", path)),
        Err(e) => Response::err(format!("Failed to set wallpaper: {}", e)),
    }
}

pub fn run_action(
    ctx: &mut crate::contexts::WmCtx<'_>,
    name: String,
    args: Vec<String>,
) -> Response {
    let action = match crate::actions::NamedAction::parse(&name, &args) {
        Ok(action) => crate::actions::KeyAction::Named(action),
        Err(error) => return Response::err(error),
    };
    match crate::actions::try_execute_key_action(ctx, &action) {
        Ok(()) => Response::ok(),
        Err(error) => Response::err(error),
    }
}

pub fn update_status(ctx: &mut crate::contexts::WmCtx<'_>, text: String) -> Response {
    ctx.core_mut().bar.set_status_text(&text);
    Response::ok()
}

pub fn get_status(ctx: &crate::contexts::WmCtx<'_>) -> Response {
    let backend = match ctx.backend_kind() {
        crate::backend::BackendKind::X11 => "x11",
        crate::backend::BackendKind::Wayland => "wayland",
    };

    let info = crate::ipc_types::WmStatusInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: crate::ipc_types::IPC_PROTOCOL_VERSION.to_string(),
        build_commit: env!("INSTANTWM_BUILD_COMMIT").to_string(),
        backend: backend.to_string(),
        running: ctx.core().is_running(),
        monitors: ctx.core().model().monitors.len(),
        windows: ctx.core().model().client_count(),
        tags: ctx.core().model().tags.num_tags,
    };

    Response::Status(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestWm as Wm;

    #[test]
    fn run_action_reports_parser_and_argument_errors() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());

        assert!(matches!(
            wm.with_ctx(|wm| run_action(wm, "missing".to_string(), Vec::new())),
            Response::Err(message) if message.contains("unknown action")
        ));
        assert!(matches!(
            wm.with_ctx(|wm| run_action(
                wm,
                "config_toggle".to_string(),
                vec!["a".to_string(), "b".to_string()],
            )),
            Response::Err(message) if message.contains("expected 1 argument")
        ));
        assert!(matches!(
            wm.with_ctx(|wm| run_action(
                wm,
                "config_toggle".to_string(),
                vec!["layout.inner_gap".to_string()],
            )),
            Response::Err(message) if message.contains("only works on boolean options")
        ));
    }

    #[test]
    fn run_action_is_the_ipc_path_for_idempotent_config_updates() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());

        for _ in 0..2 {
            assert!(matches!(
                wm.with_ctx(|wm| run_action(
                    wm,
                    "config_set".to_string(),
                    vec!["tags.show_icons".to_string(), "true".to_string()],
                )),
                Response::Ok
            ));
        }
        assert!(wm.core.state.config.tags.show_icons);
    }
}
