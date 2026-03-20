use crate::contexts::WmCtx;
use crate::ipc_types::{IpcResponse, ModeCommand};
use crate::keyboard::grab_keys_x11;
use crate::wm::Wm;

pub fn handle_mode_command(wm: &mut Wm, cmd: ModeCommand) -> IpcResponse {
    match cmd {
        ModeCommand::List => {
            let modes = &wm.g.cfg.modes;
            let current_mode = &wm.g.behavior.current_mode;

            if modes.is_empty() {
                return IpcResponse::ok("No modes configured");
            }

            let mut output = String::new();
            for (name, mode) in modes {
                let marker = if name == current_mode { "*" } else { " " };
                let desc = mode.description.as_deref().unwrap_or("(no description)");
                output.push_str(&format!("{} {} - {}\n", marker, name, desc));
            }
            IpcResponse::ok(output)
        }
        ModeCommand::Set(name) => {
            // Check if mode exists
            if !wm.g.cfg.modes.contains_key(&name) && name != "default" {
                return IpcResponse::err(format!("Mode '{}' not found", name));
            }
            wm.g.behavior.current_mode = name.clone();

            if let WmCtx::X11(x11) = wm.ctx() {
                grab_keys_x11(&x11.core, &x11.x11, x11.x11_runtime);
            }

            // Request bar update to reflect mode change
            wm.bar.mark_dirty();
            IpcResponse::ok(format!("Switched to mode '{}'", name))
        }
        ModeCommand::Toggle(name) => {
            // Check if mode exists
            if !wm.g.cfg.modes.contains_key(&name) && name != "default" {
                return IpcResponse::err(format!("Mode '{}' not found", name));
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
            IpcResponse::ok(format!("Toggled mode, now in '{}'", new_mode))
        }
    }
}
