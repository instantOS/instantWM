use crate::ipc_types::{IpcResponse, KeyboardCommand};
use crate::keyboard_layout;
use crate::wm::Wm;

pub fn handle_keyboard_command(wm: &mut Wm, cmd: KeyboardCommand) -> IpcResponse {
    let mut ctx = wm.ctx();
    match cmd {
        KeyboardCommand::Next => {
            let status = keyboard_layout::cycle_keyboard_layout(&mut ctx, true);
            IpcResponse::ok(status)
        }
        KeyboardCommand::Prev => {
            let status = keyboard_layout::cycle_keyboard_layout(&mut ctx, false);
            IpcResponse::ok(status)
        }
        KeyboardCommand::Status => {
            let status = keyboard_layout::keyboard_layout_status(&ctx);
            IpcResponse::ok(status)
        }
        KeyboardCommand::List => {
            let list = keyboard_layout::keyboard_layout_list(&ctx);
            IpcResponse::ok(list)
        }
        KeyboardCommand::ListAll => {
            let layouts = keyboard_layout::get_all_keyboard_layouts();
            let list = layouts.join("\n");
            IpcResponse::ok(list)
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
            IpcResponse::ok("")
        }
        KeyboardCommand::Add(layout) => {
            let globals_layout = crate::globals::KeyboardLayout {
                name: layout.name,
                variant: layout.variant,
            };
            match keyboard_layout::add_keyboard_layout(&mut ctx, globals_layout) {
                Ok(()) => IpcResponse::ok(""),
                Err(e) => IpcResponse::err(e),
            }
        }
        KeyboardCommand::Remove(layout) => {
            match keyboard_layout::remove_keyboard_layout(&mut ctx, &layout) {
                Ok(()) => IpcResponse::ok(""),
                Err(e) => IpcResponse::err(e),
            }
        }
    }
}
