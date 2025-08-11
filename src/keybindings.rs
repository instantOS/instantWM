use crate::State;
use smithay::input::keyboard::keysyms;
use smithay::reexports::winit::event::ModifiersState;

pub fn handle_key_event(state: &mut State, keysym: u32, modifiers: ModifiersState) {
    if modifiers.contains(ModifiersState::SUPER) {
        match keysym {
            keysyms::KEY_1 => state.active_workspace = 0,
            keysyms::KEY_2 => state.active_workspace = 1,
            keysyms::KEY_3 => state.active_workspace = 2,
            keysyms::KEY_4 => state.active_workspace = 3,
            keysyms::KEY_5 => state.active_workspace = 4,
            keysyms::KEY_6 => state.active_workspace = 5,
            keysyms::KEY_7 => state.active_workspace = 6,
            keysyms::KEY_8 => state.active_workspace = 7,
            keysyms::KEY_9 => state.active_workspace = 8,
            _ => {}
        }
    }
}
