use crate::globals::*;
use crate::types::*;

// TODO: Port keyboard handling from keyboard.c

pub fn key_press(_e: &x11rb::protocol::xproto::KeyPressEvent) {
    // TODO: Handle key press
}

pub fn key_release(_e: &x11rb::protocol::xproto::KeyReleaseEvent) {
    // TODO: Handle key release
}

pub fn grab_keys() {
    // TODO: Grab keys for all keybindings
}

pub fn up_key(_arg: &Arg) {
    // TODO: Focus window above
}

pub fn down_key(_arg: &Arg) {
    // TODO: Focus window below
}

pub fn space_toggle(_arg: &Arg) {
    // TODO: Toggle space
}

pub fn key_resize(_arg: &Arg) {
    // TODO: Resize with keyboard
}

pub fn center_window(_arg: &Arg) {
    // TODO: Center floating window
}

pub fn focus_stack(_arg: &Arg) {
    // TODO: Focus next/prev in stack
}

pub fn focus_mon(_arg: &Arg) {
    // TODO: Focus monitor by number
}

pub fn focus_nmon(_arg: &Arg) {
    // TODO: Focus next/prev monitor
}

pub fn follow_mon(_arg: &Arg) {
    // TODO: Move focused window to monitor
}

pub fn update_num_lock_mask() {
    // TODO: Update numlock mask
}
