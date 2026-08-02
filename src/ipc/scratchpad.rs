use crate::floating::scratchpad::{
    collect_scratchpad_info, scratchpad_create, scratchpad_hide_all, scratchpad_hide_name,
    scratchpad_resize_name, scratchpad_restore, scratchpad_show_all, scratchpad_show_name,
    scratchpad_toggle,
};
use crate::ipc_types::{Response, ScratchpadCommand};
use crate::types::WindowId;
use crate::types::input::EdgeDirection;
use crate::wm::Wm;

pub fn handle_scratchpad_command(wm: &mut Wm, cmd: ScratchpadCommand) -> Response {
    match cmd {
        ScratchpadCommand::List => {
            let scratchpads = collect_scratchpad_info(&wm.core.model);
            Response::ScratchpadList(scratchpads)
        }
        ScratchpadCommand::Toggle(name) => {
            scratchpad_toggle(&mut wm.ctx(), name.as_deref());
            Response::ok()
        }
        ScratchpadCommand::Show(name) => {
            if let Some(n) = name {
                match scratchpad_show_name(&mut wm.ctx(), &n) {
                    Ok(msg) => Response::Message(msg),
                    Err(e) => Response::err(e),
                }
            } else {
                Response::err("scratchpad name required (or use --all)")
            }
        }
        ScratchpadCommand::ShowAll => match scratchpad_show_all(&mut wm.ctx()) {
            Some(msg) => Response::Message(msg),
            None => Response::ok(),
        },
        ScratchpadCommand::Hide(name) => {
            if let Some(name) = name {
                scratchpad_hide_name(&mut wm.ctx(), &name);
                Response::ok()
            } else {
                Response::err("scratchpad name required (or use --all)")
            }
        }
        ScratchpadCommand::HideAll => match scratchpad_hide_all(&mut wm.ctx()) {
            Some(msg) => Response::Message(msg),
            None => Response::ok(),
        },
        ScratchpadCommand::Status(name) => {
            let mut scratchpads = collect_scratchpad_info(&wm.core.model);
            if let Some(ref n) = name {
                scratchpads.retain(|sp| sp.name == *n);
            }
            Response::ScratchpadList(scratchpads)
        }
        ScratchpadCommand::Resize {
            name,
            width_percent,
            height_percent,
        } => match scratchpad_resize_name(&mut wm.ctx(), &name, width_percent, height_percent) {
            Ok(message) => Response::Message(message),
            Err(error) => Response::err(error),
        },
        ScratchpadCommand::Create {
            name,
            window_id,
            status,
            direction,
        } => {
            let dir = match direction {
                Some(direction) => match EdgeDirection::from_str_loose(&direction) {
                    Some(direction) => Some(direction),
                    None => {
                        return Response::err(format!(
                            "invalid scratchpad direction '{}'; expected top, bottom, left, or right",
                            direction
                        ));
                    }
                },
                None => None,
            };
            match scratchpad_create(
                &mut wm.ctx(),
                &name,
                window_id.map(WindowId::from),
                dir,
                status,
            ) {
                Ok(message) => Response::Message(message),
                Err(error) => Response::err(error),
            }
        }
        ScratchpadCommand::Restore { name, window_id } => match scratchpad_restore(
            &mut wm.ctx(),
            name.as_deref(),
            window_id.map(WindowId::from),
        ) {
            Ok(message) => Response::Message(message),
            Err(error) => Response::err(error),
        },
    }
}
