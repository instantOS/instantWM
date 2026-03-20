use crate::config::config_toml::{AccelProfile, ToggleSetting};
use crate::ipc_types::{InputCommand, IpcResponse};
use crate::wm::Wm;

pub fn handle_input_command(wm: &mut Wm, cmd: InputCommand) -> IpcResponse {
    let inputs = &mut wm.g.cfg.input;
    match cmd {
        InputCommand::List(identifier) => {
            let entries: Vec<_> = match &identifier {
                Some(id) => inputs
                    .iter()
                    .filter(|(k, _)| k.as_str() == id.as_str())
                    .collect(),
                None => inputs.iter().collect(),
            };
            if entries.is_empty() {
                return IpcResponse::ok(
                    "no input configuration found\n\nHint: Use 'instantwmctl mouse devices' to see connected physical devices.\n      Common identifiers are 'type:pointer', 'type:touchpad', or '*'.",
                );
            }
            let info: Vec<String> = entries
                .iter()
                .map(|(id, cfg)| {
                    format!(
                        "[{}]\ntap: {:?}\nnatural_scroll: {:?}\naccel_profile: {:?}\npointer_accel: {:?}\nscroll_factor: {:?}",
                        id, cfg.tap, cfg.natural_scroll, cfg.accel_profile, cfg.pointer_accel, cfg.scroll_factor,
                    )
                })
                .collect();
            return IpcResponse::ok(info.join("\n\n"));
        }
        InputCommand::Devices => {
            let devices = wm.backend.get_input_devices();
            if devices.is_empty() {
                return IpcResponse::ok("no input devices detected (or not supported by backend)");
            }
            return IpcResponse::ok(devices.join("\n"));
        }
        InputCommand::PointerAccel { identifier, value } => {
            let identifier = identifier.unwrap_or_else(|| "*".to_string());
            let cfg = inputs.entry(identifier).or_default();
            cfg.pointer_accel = Some(value.clamp(-1.0, 1.0));
        }
        InputCommand::AccelProfile {
            identifier,
            profile,
        } => {
            let p = match profile.to_lowercase().as_str() {
                "flat" => AccelProfile::Flat,
                "adaptive" => AccelProfile::Adaptive,
                _ => {
                    return IpcResponse::err(format!(
                        "unknown accel profile '{profile}' (expected 'flat' or 'adaptive')"
                    ));
                }
            };
            let identifier = identifier.unwrap_or_else(|| "*".to_string());
            let cfg = inputs.entry(identifier).or_default();
            cfg.accel_profile = Some(p);
        }
        InputCommand::Tap {
            identifier,
            enabled,
        } => {
            let identifier = identifier.unwrap_or_else(|| "*".to_string());
            let cfg = inputs.entry(identifier).or_default();
            cfg.tap = Some(if enabled {
                ToggleSetting::Enabled
            } else {
                ToggleSetting::Disabled
            });
        }
        InputCommand::NaturalScroll {
            identifier,
            enabled,
        } => {
            let identifier = identifier.unwrap_or_else(|| "*".to_string());
            let cfg = inputs.entry(identifier).or_default();
            cfg.natural_scroll = Some(if enabled {
                ToggleSetting::Enabled
            } else {
                ToggleSetting::Disabled
            });
        }
        InputCommand::ScrollFactor { identifier, value } => {
            let identifier = identifier.unwrap_or_else(|| "*".to_string());
            let cfg = inputs.entry(identifier).or_default();
            cfg.scroll_factor = Some(value);
        }
    }
    wm.g.dirty.input_config = true;
    IpcResponse::ok("")
}
