use crate::config::config_toml::{AccelProfile, ToggleSetting};
use crate::ipc_types::{InputCommand, Response};
use crate::wm::Wm;

pub fn handle_input_command(wm: &mut Wm, cmd: InputCommand) -> Response {
    let inputs = &mut wm.g.cfg.input;
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
                .map(|(id, cfg)| {
                    format!(
                        "[{}]\ntap: {:?}\nnatural_scroll: {:?}\naccel_profile: {:?}\npointer_accel: {:?}\nscroll_factor: {:?}\nleft_handed: {:?}",
                        id, cfg.tap, cfg.natural_scroll, cfg.accel_profile, cfg.pointer_accel, cfg.scroll_factor, cfg.left_handed,
                    )
                })
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
                    return Response::err(format!(
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
        InputCommand::LeftHanded {
            identifier,
            enabled,
        } => {
            let identifier = identifier.unwrap_or_else(|| "*".to_string());
            let cfg = inputs.entry(identifier).or_default();
            cfg.left_handed = Some(if enabled {
                ToggleSetting::Enabled
            } else {
                ToggleSetting::Disabled
            });
        }
    }
    wm.g.dirty.input_config = true;
    Response::ok()
}
