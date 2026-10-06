use crate::contexts::WmCtx;
use crate::ipc_types::{KeyboardCommand, KeyboardLayoutInfo, Response};
use crate::keyboard_layout;
use crate::types::StackDirection;

pub fn handle_keyboard_command(ctx: &mut WmCtx<'_>, cmd: KeyboardCommand) -> Response {
    match cmd {
        KeyboardCommand::Status => {
            let status = ctx.interaction().keyboard_layout.status();
            Response::Message(status)
        }
        KeyboardCommand::List { all: true } => {
            Response::Message(keyboard_layout::get_all_keyboard_layouts().join("\n"))
        }
        KeyboardCommand::List { all: false } => {
            let state = &ctx.interaction().keyboard_layout;
            let layouts: Vec<KeyboardLayoutInfo> = state
                .layouts
                .iter()
                .enumerate()
                .map(|(i, l)| KeyboardLayoutInfo {
                    name: l.name.clone(),
                    variant: l.variant.clone(),
                    is_active: i == state.current,
                })
                .collect();
            Response::KeyboardLayoutList(layouts)
        }
        KeyboardCommand::Next | KeyboardCommand::Prev => {
            let direction = if matches!(cmd, KeyboardCommand::Next) {
                StackDirection::Next
            } else {
                StackDirection::Previous
            };
            let _ = keyboard_layout::cycle_keyboard_layout(ctx, direction);
            Response::ok()
        }
        KeyboardCommand::Set { layouts } => {
            keyboard_layout::set_keyboard_layouts(ctx, layouts);
            Response::ok()
        }
        KeyboardCommand::Add { layout } => {
            match keyboard_layout::add_keyboard_layout(ctx, layout) {
                Ok(()) => Response::ok(),
                Err(e) => Response::err(e),
            }
        }
        KeyboardCommand::Remove { layout } => {
            match keyboard_layout::remove_keyboard_layout(ctx, &layout) {
                Ok(()) => Response::ok(),
                Err(e) => Response::err(e),
            }
        }
        KeyboardCommand::SwapEscape { enabled } => {
            keyboard_layout::set_swapescape(ctx, enabled);
            Response::ok()
        }
    }
}
