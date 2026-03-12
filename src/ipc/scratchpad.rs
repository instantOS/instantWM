use crate::ipc_types::{IpcResponse, ScratchpadCommand};
use crate::scratchpad::{
    scratchpad_hide_name, scratchpad_list, scratchpad_make, scratchpad_show_name,
    scratchpad_status, scratchpad_toggle, scratchpad_unmake,
};
use crate::wm::Wm;

pub fn handle_scratchpad_command(wm: &mut Wm, cmd: ScratchpadCommand) -> IpcResponse {
    match cmd {
        ScratchpadCommand::List => {
            let list = scratchpad_list(&wm.g);
            IpcResponse::ok(list)
        }
        ScratchpadCommand::Toggle(name) => {
            scratchpad_toggle(&mut wm.ctx(), name.as_deref());
            IpcResponse::ok("")
        }
        ScratchpadCommand::Show(name) => {
            scratchpad_show_name(&mut wm.ctx(), &name);
            IpcResponse::ok("")
        }
        ScratchpadCommand::Hide(name) => {
            scratchpad_hide_name(&mut wm.ctx(), &name);
            IpcResponse::ok("")
        }
        ScratchpadCommand::Status(name) => {
            let status = scratchpad_status(&wm.g, name.as_deref().unwrap_or(""));
            IpcResponse::ok(status)
        }
        ScratchpadCommand::Create(name) => {
            scratchpad_make(&mut wm.ctx(), name.as_deref());
            IpcResponse::ok("")
        }
        ScratchpadCommand::Delete => {
            scratchpad_unmake(&mut wm.ctx());
            IpcResponse::ok("")
        }
    }
}
