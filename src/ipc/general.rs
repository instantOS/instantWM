use crate::ipc_types::Response;
use crate::wm::Wm;

pub fn set_wallpaper(wm: &mut Wm, path: String) -> Response {
    match wm.backend.set_wallpaper(&path) {
        Ok(()) => Response::Message(format!("Wallpaper set to {}", path)),
        Err(e) => Response::err(format!("Failed to set wallpaper: {}", e)),
    }
}

pub fn run_action(wm: &mut Wm, name: String, args: Vec<String>) -> Response {
    let action = match crate::actions::NamedAction::parse(&name, &args) {
        Ok(action) => crate::actions::KeyAction::Named(action),
        Err(error) => return Response::err(error),
    };
    match crate::actions::try_execute_key_action(&mut wm.ctx(), &action) {
        Ok(()) => Response::ok(),
        Err(error) => Response::err(error),
    }
}

pub fn update_status(wm: &mut Wm, text: String) -> Response {
    wm.bar.set_status_text(&text);
    Response::ok()
}

pub fn get_status(wm: &Wm) -> Response {
    let backend = match &wm.backend {
        crate::backend::Backend::X11(_) => "x11",
        crate::backend::Backend::Wayland(_) => "wayland",
    };

    let info = crate::ipc_types::WmStatusInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: crate::ipc_types::IPC_PROTOCOL_VERSION.to_string(),
        build_commit: env!("INSTANTWM_BUILD_COMMIT").to_string(),
        backend: backend.to_string(),
        running: wm.running,
        monitors: wm.core.model.monitors.len(),
        windows: wm.core.model.clients.len(),
        tags: wm.core.model.tags.num_tags,
    };

    Response::Status(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Backend, wayland::WaylandBackend};

    #[test]
    fn run_action_reports_parser_and_argument_errors() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));

        assert!(matches!(
            run_action(&mut wm, "missing".to_string(), Vec::new()),
            Response::Err(message) if message.contains("unknown action")
        ));
        assert!(matches!(
            run_action(
                &mut wm,
                "config_toggle".to_string(),
                vec!["a".to_string(), "b".to_string()],
            ),
            Response::Err(message) if message.contains("expected 1 argument")
        ));
        assert!(matches!(
            run_action(
                &mut wm,
                "config_toggle".to_string(),
                vec!["layout.inner_gap".to_string()],
            ),
            Response::Err(message) if message.contains("only works on boolean options")
        ));
    }

    #[test]
    fn run_action_is_the_ipc_path_for_idempotent_config_updates() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));

        for _ in 0..2 {
            assert!(matches!(
                run_action(
                    &mut wm,
                    "config_set".to_string(),
                    vec!["tags.show_icons".to_string(), "true".to_string()],
                ),
                Response::Ok
            ));
        }
        assert!(wm.core.config.tags.show_icons);
    }
}
