use crate::types::*;

pub fn motion_notify(_e: &x11rb::protocol::xproto::MotionNotifyEvent) {}

pub fn button_press(_e: &x11rb::protocol::xproto::ButtonPressEvent) {}

pub fn move_resize(_arg: &Arg) {}

pub fn moveresize(_arg: &Arg) {}

pub fn get_cursor_client() -> Option<ClientInner> {
    None
}

pub fn warp(_c: &ClientInner) {}

pub fn force_warp(_c: &ClientInner) {}

pub fn warp_cursor_to_client(_c: &ClientInner) {}

pub fn warp_to_focus(_arg: &Arg) {}

pub fn reset_cursor() {}

pub fn grab_buttons(_c: &mut ClientInner, _focused: bool) {}
