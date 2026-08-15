use crate::ipc_types::{Response, TagCommand};
use crate::tags::{name_tag, reset_name_tag};
use crate::wm::Wm;

pub fn handle_tag_command(wm: &mut Wm, cmd: TagCommand) -> Response {
    match cmd {
        TagCommand::Name(name) => name_tag_cmd(wm, name),
        TagCommand::ResetNames => reset_tag_names(wm),
    }
}

fn name_tag_cmd(wm: &mut Wm, name: String) -> Response {
    name_tag(&mut wm.ctx(), &name);
    Response::ok()
}

fn reset_tag_names(wm: &mut Wm) -> Response {
    reset_name_tag(&mut wm.ctx());
    Response::ok()
}
