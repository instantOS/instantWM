use crate::floating::scratchpad::{
    scratchpad_hide_name, scratchpad_list, scratchpad_make, scratchpad_show_all,
    scratchpad_show_name, scratchpad_status, scratchpad_toggle, scratchpad_unmake,
};
use crate::ipc_types::{Response, ScratchpadCommand, ScratchpadInfo};
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
            let status = scratchpad_status(&wm.g, name.as_deref().unwrap_or(""));
            Response::Message(status)
        }
        ScratchpadCommand::Create(name) => {
            scratchpad_make(&mut wm.ctx(), name.as_deref());
            Response::ok()
        }
        ScratchpadCommand::Delete => {
            scratchpad_unmake(&mut wm.ctx());
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
