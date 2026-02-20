use crate::types::*;

// TODO: Port mouse handling from mouse.c

pub fn motion_notify(_e: &x11rb::protocol::xproto::MotionNotifyEvent) {
    // TODO: Handle mouse motion
}

pub fn button_press(_e: &x11rb::protocol::xproto::ButtonPressEvent) {
    // TODO: Handle button press
}

pub fn move_resize(_arg: &Arg) {
    // TODO: Move/resize with mouse
}

pub fn moveresize(_arg: &Arg) {
    // TODO: Move/resize client
}

pub fn get_cursor_client() -> Option<Client> {
    // TODO: Get client under cursor
    None
}

pub fn warp(_c: &Client) {
    // TODO: Warp cursor to client
}

pub fn force_warp(_c: &Client) {
    // TODO: Force warp cursor to client
}

pub fn warp_cursor_to_client(_c: &Client) {
    // TODO: Warp cursor to center of client
}

pub fn warp_to_focus(_arg: &Arg) {
    // TODO: Warp cursor to focused client
}

pub fn reset_cursor() {
    // TODO: Reset cursor to default
}

pub fn grab_buttons(_c: &mut Client, _focused: bool) {
    // TODO: Grab mouse buttons for client
}
