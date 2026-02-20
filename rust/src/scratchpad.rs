use crate::types::*;

pub fn scratchpad_toggle(_arg: &Arg) {}

pub fn scratchpad_make(_arg: &Arg) {}

pub fn scratchpad_unmake(_arg: &Arg) {}

pub fn scratchpad_show(_arg: &Arg) {}

pub fn scratchpad_hide(_arg: &Arg) {}

pub fn scratchpad_status(_arg: &Arg) {}

pub fn scratchpad_identify_client(_c: &ClientInner) -> bool {
    false
}

pub const ISSCRATCHPAD: fn(&ClientInner) -> bool = scratchpad_identify_client;

pub fn hide_window(_win: x11rb::protocol::xproto::Window) {}

pub fn unhide_one() -> bool {
    false
}
