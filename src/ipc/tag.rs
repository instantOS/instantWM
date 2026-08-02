use crate::ipc_types::{Response, TagCommand};
use crate::tags::{name_tag, reset_name_tag};
use crate::wm::Wm;

pub fn handle_tag_command(wm: &mut Wm, cmd: TagCommand) -> Response {
    match cmd {
        TagCommand::View(tag_num) => view_tag(wm, tag_num),
        TagCommand::Name(name) => name_tag_cmd(wm, name),
        TagCommand::ResetNames => reset_tag_names(wm),
    }
}

fn view_tag(wm: &mut Wm, tag_num: u32) -> Response {
    let tag = if tag_num == 0 { 2 } else { tag_num };
    if let Some(tag_idx) = usize::try_from(tag).ok().and_then(|tag| tag.checked_sub(1)) {
        crate::actions::execute_key_action(
            &mut wm.ctx(),
            &crate::actions::KeyAction::ViewTag { tag_idx },
        );
    }
    Response::ok()
}

fn name_tag_cmd(wm: &mut Wm, name: String) -> Response {
    name_tag(&mut wm.ctx(), &name);
    Response::ok()
}

fn reset_tag_names(wm: &mut Wm) -> Response {
    reset_name_tag(&mut wm.ctx());
    Response::ok()
}
