use crate::config::config_toml::{AccelProfile, InputConfig};
use crate::ipc_types::{InputCommand, Response};
use crate::wm::Wm;
use std::collections::HashMap;

fn input_config_mut(
    inputs: &mut HashMap<String, InputConfig>,
    identifier: Option<String>,
) -> &mut InputConfig {
    inputs
        .entry(identifier.unwrap_or_else(|| "*".into()))
        .or_default()
}

pub fn handle_input_command(wm: &mut Wm, cmd: InputCommand) -> Response {
    let inputs = &mut wm.core.config.input;
    match cmd {
        InputCommand::List(identifier) => {
            let mut entries: Vec<(String, &crate::config::config_toml::InputConfig)> =
                match &identifier {
                    Some(id) => inputs
                        .iter()
                        .filter(|(k, _)| k.as_str() == id.as_str())
                        .map(|(k, v)| (k.clone(), v))
                        .collect(),
                    None => inputs.iter().map(|(k, v)| (k.clone(), v)).collect(),
                };

            let show_defaults = match &identifier {
                Some(id) => id == "*" && entries.is_empty(),
                None => entries.is_empty(),
            };

            let default_cfg = crate::config::config_toml::InputConfig::default();
            if show_defaults {
                entries.push(("*".to_string(), &default_cfg));
            }

            if entries.is_empty() {
                return Response::Message(format!(
                    "no input configuration found for '{}'\n\nHint: Use 'instantwmctl mouse devices' to see connected physical devices.\n      Common identifiers are 'type:pointer', 'type:touchpad', or '*'.",
                    identifier.unwrap_or_default()
                ));
            }
            let info: Vec<String> = entries
                .iter()
                .map(|(id, cfg)| format!("[{}]\n{}", id, cfg))
                .collect();
            return Response::Message(info.join("\n\n"));
        }
        InputCommand::Devices => {
            let devices = wm.backend.get_input_devices();
            if devices.is_empty() {
                return Response::Message(
                    "no input devices detected (or not supported by backend)".to_string(),
                );
            }
            return Response::Message(devices.join("\n"));
        }
        InputCommand::PointerAccel { identifier, value } => {
            input_config_mut(inputs, identifier).pointer_accel = Some(value.clamp(-1.0, 1.0));
        }
        InputCommand::AccelProfile {
            identifier,
            profile,
        } => {
            let p = match profile.to_lowercase().as_str() {
                "flat" => AccelProfile::Flat,
                "adaptive" => AccelProfile::Adaptive,
                _ => {
                    return Response::err(format!(
                        "unknown accel profile '{profile}' (expected 'flat' or 'adaptive')"
                    ));
                }
            };
            input_config_mut(inputs, identifier).accel_profile = Some(p);
        }
        InputCommand::Tap {
            identifier,
            enabled,
        } => {
            input_config_mut(inputs, identifier).tap = Some(enabled.into());
        }
        InputCommand::NaturalScroll {
            identifier,
            enabled,
        } => {
            input_config_mut(inputs, identifier).natural_scroll = Some(enabled.into());
        }
        InputCommand::ScrollFactor { identifier, value } => {
            input_config_mut(inputs, identifier).scroll_factor = Some(value);
        }
        InputCommand::LeftHanded {
            identifier,
            enabled,
        } => {
            input_config_mut(inputs, identifier).left_handed = Some(enabled.into());
        }
    }
    wm.work.queue_input_config_apply();
    Response::ok()
}
