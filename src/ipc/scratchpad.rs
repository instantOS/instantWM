use crate::floating::scratchpad::{
    scratchpad_hide_name, scratchpad_make, scratchpad_show_all, scratchpad_show_name,
    scratchpad_toggle, scratchpad_unmake,
};
use crate::ipc_types::{Response, ScratchpadCommand, ScratchpadInfo};
use crate::types::WindowId;
use crate::wm::Wm;

pub fn handle_scratchpad_command(wm: &mut Wm, cmd: ScratchpadCommand) -> Response {
    match cmd {
        ScratchpadCommand::List => {
            let scratchpads = collect_scratchpad_info(&wm.g);
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
            scratchpad_hide_name(&mut wm.ctx(), &name);
            Response::ok()
        }
        ScratchpadCommand::Status(name) => {
            let mut scratchpads = collect_scratchpad_info(&wm.g);
            if let Some(ref n) = name {
                scratchpads.retain(|sp| sp.name == *n);
            }
            Response::ScratchpadList(scratchpads)
        }
        ScratchpadCommand::Create {
            name,
            window_id,
            status,
        } => {
            scratchpad_make(&mut wm.ctx(), &name, window_id.map(WindowId::from), status);
            Response::ok()
        }
        ScratchpadCommand::Delete { window_id } => {
            scratchpad_unmake(&mut wm.ctx(), window_id.map(WindowId::from));
            Response::ok()
        }
    }
}

fn collect_scratchpad_info(g: &crate::globals::Globals) -> Vec<ScratchpadInfo> {
    let mut scratchpads = Vec::new();

    for mon in g.monitors_iter_all() {
        for (c_win, c) in mon.iter_clients(g.clients.map()) {
            if c.is_scratchpad() {
                scratchpads.push(ScratchpadInfo {
                    name: c.scratchpad_name.clone(),
                    visible: c.issticky,
                    window_id: Some(c_win.0),
                    monitor: Some(c.monitor_id),
                    x: Some(c.geo.x),
                    y: Some(c.geo.y),
                    width: Some(c.geo.w),
                    height: Some(c.geo.h),
                    floating: c.is_floating,
                    fullscreen: c.is_fullscreen,
                });
            }
        }
    }

    scratchpads
}
