use crate::ipc_types::Response;
use crate::wm::Wm;
use std::process::Command;

pub fn set_wallpaper(wm: &mut Wm, path: String) -> Response {
    if wm.ctx().is_wayland() {
        let _ = Command::new("killall").arg("swaybg").status();
        let status = Command::new("swaybg")
            .arg("-i")
            .arg(&path)
            .arg("-m")
            .arg("fill")
            .spawn();
        match status {
            Ok(_) => Response::Message(format!("Wallpaper set to {}", path)),
            Err(e) => Response::err(format!("Failed to spawn swaybg: {}", e)),
        }
    } else {
        let status = Command::new("feh").arg("--bg-fill").arg(&path).spawn();
        match status {
            Ok(_) => Response::Message(format!("Wallpaper set to {}", path)),
            Err(e) => Response::err(format!("Failed to spawn feh: {}", e)),
        }
    }
}

pub fn run_action(wm: &mut Wm, name: String, args: Vec<String>) -> Response {
    let Some(action) = crate::actions::parse_named_action(&name) else {
        return Response::err(format!("unknown action '{name}'"));
    };
    let action = crate::actions::KeyAction::Named { action, args };
    match crate::actions::try_execute_key_action(&mut wm.ctx(), &action) {
        Ok(()) => Response::ok(),
        Err(error) => Response::err(error),
    }
}

pub fn update_status(wm: &mut Wm, text: String) -> Response {
    crate::bar::status::apply_status_update(wm, text);
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
                "toggle_alt_tag".to_string(),
                vec!["invalid".to_string()],
            ),
            Response::Err(message) if message.contains("expected toggle, on, or off")
        ));
    }

    #[test]
    fn run_action_is_the_ipc_path_for_idempotent_toggle_updates() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));

        for _ in 0..2 {
            assert!(matches!(
                run_action(
                    &mut wm,
                    "toggle_alt_tag".to_string(),
                    vec!["on".to_string()],
                ),
                Response::Ok
            ));
        }
        assert!(wm.core.model.tags.show_alternative_names);
    }
}
