use crate::floating::scratchpad::{
    collect_scratchpad_info, scratchpad_create, scratchpad_hide_all, scratchpad_hide_name,
    scratchpad_resize_name, scratchpad_restore, scratchpad_show_all, scratchpad_show_name,
    scratchpad_toggle,
};
use crate::ipc_types::{Response, ScratchpadCommand};
use crate::types::WindowId;

fn message_or_err(result: Result<String, String>) -> Response {
    match result {
        Ok(message) => Response::Message(message),
        Err(error) => Response::err(error),
    }
}

fn message_or_ok(message: Option<String>) -> Response {
    message.map_or(Response::Ok, Response::Message)
}

pub fn handle_scratchpad_command(
    ctx: &mut crate::contexts::WmCtx<'_>,
    cmd: ScratchpadCommand,
) -> Response {
    match cmd {
        ScratchpadCommand::Status { name } => {
            let mut scratchpads = collect_scratchpad_info(ctx.core().model());
            if let Some(name) = name {
                scratchpads.retain(|sp| sp.name == name);
            }
            Response::ScratchpadList(scratchpads)
        }
        ScratchpadCommand::Toggle { name } => {
            scratchpad_toggle(ctx, Some(&name));
            Response::ok()
        }
        ScratchpadCommand::Show { all: true, .. } => message_or_ok(scratchpad_show_all(ctx)),
        ScratchpadCommand::Show { name, .. } => message_or_err(scratchpad_show_name(ctx, &name)),
        ScratchpadCommand::Hide { all: true, .. } => message_or_ok(scratchpad_hide_all(ctx)),
        ScratchpadCommand::Hide { name, .. } => {
            scratchpad_hide_name(ctx, &name);
            Response::ok()
        }
        ScratchpadCommand::Resize {
            name,
            width,
            height,
        } => message_or_err(scratchpad_resize_name(ctx, &name, width, height)),
        ScratchpadCommand::Create {
            name,
            window_id,
            status,
            direction,
        } => message_or_err(scratchpad_create(
            ctx,
            &name,
            window_id.map(WindowId::from),
            direction,
            status,
        )),
        ScratchpadCommand::Restore { name, window_id } => message_or_err(scratchpad_restore(
            ctx,
            name.as_deref(),
            window_id.map(WindowId::from),
        )),
    }
}
