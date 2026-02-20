use crate::types::*;

pub fn key_press(_e: &x11rb::protocol::xproto::KeyPressEvent) {}

pub fn key_release(_e: &x11rb::protocol::xproto::KeyReleaseEvent) {}

pub fn grab_keys() {}

pub fn up_key(_arg: &Arg) {}

pub fn down_key(_arg: &Arg) {}

pub fn space_toggle(_arg: &Arg) {}

pub fn key_resize(_arg: &Arg) {}

pub fn center_window(_arg: &Arg) {}

pub fn focus_stack(_arg: &Arg) {}

pub fn focus_mon(_arg: &Arg) {}

pub fn focus_nmon(_arg: &Arg) {}

pub fn follow_mon(_arg: &Arg) {}

pub fn update_num_lock_mask() {}
