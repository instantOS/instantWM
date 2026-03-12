use crate::ipc_types::{IpcResponse, TagCommand};
use crate::tags::{name_tag, reset_name_tag};
use crate::types::TagMask;
use crate::wm::Wm;

pub fn handle_tag_command(wm: &mut Wm, cmd: TagCommand) -> IpcResponse {
    match cmd {
        TagCommand::View(tag_num) => view_tag(wm, tag_num),
        TagCommand::Name(name) => name_tag_cmd(wm, name),
        TagCommand::ResetNames => reset_tag_names(wm),
    }
}

fn view_tag(wm: &mut Wm, tag_num: u32) -> IpcResponse {
    let tag = if tag_num == 0 { 2 } else { tag_num };
    if let Some(mask) = TagMask::single(tag as usize) {
        crate::tags::view::view(&mut wm.ctx(), mask);
    }
    IpcResponse::ok("")
}

fn name_tag_cmd(wm: &mut Wm, name: String) -> IpcResponse {
    name_tag(&mut wm.ctx(), &name);
    IpcResponse::ok("")
}

fn reset_tag_names(wm: &mut Wm) -> IpcResponse {
    reset_name_tag(&mut wm.ctx());
    IpcResponse::ok("")
}
