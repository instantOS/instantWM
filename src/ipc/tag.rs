use crate::ipc_types::{Response, TagCommand};
use crate::tags::{name_tag, reset_name_tag};
use crate::wm::Wm;

pub fn handle_tag_command(wm: &mut Wm, cmd: TagCommand) -> Response {
    match cmd {
        TagCommand::Name { name } => name_tag(&mut wm.ctx(), &name),
        TagCommand::Reset => reset_name_tag(&mut wm.ctx()),
    }
    Response::ok()
}
