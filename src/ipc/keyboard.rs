use crate::ipc_types::{KeyboardCommand, KeyboardLayoutInfo, Response};
use crate::keyboard_layout;
use crate::wm::Wm;

pub fn handle_keyboard_command(wm: &mut Wm, cmd: KeyboardCommand) -> Response {
    let mut ctx = wm.ctx();
    match cmd {
        KeyboardCommand::Next => {
            let status = keyboard_layout::cycle_keyboard_layout(&mut ctx, true);
            Response::Message(status)
        }
        KeyboardCommand::Prev => {
            let status = keyboard_layout::cycle_keyboard_layout(&mut ctx, false);
            Response::Message(status)
        }
        KeyboardCommand::Status => {
            let status = keyboard_layout::keyboard_layout_status(&ctx);
            Response::Message(status)
        }
        KeyboardCommand::List => {
            let state = &ctx.core().globals().keyboard_layout;
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
        KeyboardCommand::ListAll => {
            let layouts = keyboard_layout::get_all_keyboard_layouts();
            let list = layouts.join("\n");
            Response::Message(list)
        }
        KeyboardCommand::Set(layouts) => {
            let globals_layouts: Vec<crate::globals::KeyboardLayout> = layouts
                .into_iter()
                .map(|l| crate::globals::KeyboardLayout {
                    name: l.name,
                    variant: l.variant,
                })
                .collect();
            keyboard_layout::set_keyboard_layouts(&mut ctx, globals_layouts);
            Response::ok()
        }
        KeyboardCommand::Add(layout) => {
            let globals_layout = crate::globals::KeyboardLayout {
                name: layout.name,
                variant: layout.variant,
            };
            match keyboard_layout::add_keyboard_layout(&mut ctx, globals_layout) {
                Ok(()) => Response::ok(),
                Err(e) => Response::err(e),
            }
        }
        KeyboardCommand::Remove(layout) => {
            match keyboard_layout::remove_keyboard_layout(&mut ctx, &layout) {
                Ok(()) => Response::ok(),
                Err(e) => Response::err(e),
            }
        }
        KeyboardCommand::SwapEscape(enabled) => {
            keyboard_layout::set_swapescape(&mut ctx, enabled);
            Response::ok()
        }
    }
}
