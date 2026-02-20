use crate::globals::*;
use crate::types::*;

// TODO: Port client management functions from client.c

pub fn apply_rules(_c: &mut Client) {
    // TODO: Apply window rules to client
}

pub fn apply_size_hints(
    _c: &mut Client,
    _x: &mut i32,
    _y: &mut i32,
    _w: &mut i32,
    _h: &mut i32,
    _interact: bool,
) -> bool {
    // TODO: Apply size hints
    false
}

pub fn attach(_c: &mut Client, _mons: &mut Monitor) {
    // TODO: Attach client to monitor's client list
}

pub fn attach_stack(_c: &mut Client, _mons: &mut Monitor) {
    // TODO: Attach client to monitor's stack
}

pub fn detach(_c: &mut Client, _mons: &mut Monitor) {
    // TODO: Detach client from monitor's client list
}

pub fn detach_stack(_c: &mut Client, _mons: &mut Monitor) {
    // TODO: Detach client from monitor's stack
}

pub fn manage(_w: u32, _wa: &x11rb::protocol::xproto::WindowAttributes) {
    // TODO: Manage a new window
}

pub fn unmanage(_c: &mut Client, _destroyed: bool) {
    // TODO: Unmanage a client
}

pub fn resize(_c: &mut Client, _x: i32, _y: i32, _w: i32, _h: i32, _interact: bool) {
    // TODO: Resize a client
}

pub fn resize_client(_c: &mut Client, _x: i32, _y: i32, _w: i32, _h: i32) {
    // TODO: Resize client with configure notify
}

pub fn next_tiled(_c: &Option<Box<Client>>) -> Option<&Client> {
    // TODO: Get next tiled client
    None
}

pub fn pop(_c: &mut Client) {
    // TODO: Pop client to top of stack
}

pub fn set_fullscreen(_c: &mut Client, _fullscreen: bool) {
    // TODO: Set client fullscreen state
}

pub fn set_urgent(_c: &mut Client, _urgent: bool) {
    // TODO: Set client urgent state
}

pub fn focus(_c: Option<&mut Client>) {
    // TODO: Focus a client
}

pub fn unfocus(_c: &mut Client, _set_focus: bool) {
    // TODO: Unfocus a client
}

pub fn update_size_hints(_c: &mut Client) {
    // TODO: Update client size hints
}

pub fn update_wm_hints(_c: &mut Client) {
    // TODO: Update WM hints
}

pub fn update_motif_hints(_c: &mut Client) {
    // TODO: Update Motif hints
}

pub fn win_to_client(_w: u32) -> Option<Client> {
    // TODO: Find client by window
    None
}

pub fn hide(_c: &mut Client) {
    // TODO: Hide a client
}

pub fn show(_c: &mut Client) {
    // TODO: Show a hidden client
}

pub fn show_hide(_c: &mut Client) {
    // TODO: Show or hide client based on visibility
}

pub fn scale_client(_c: &mut Client, _scale: i32) {
    // TODO: Scale client dimensions
}

pub fn save_floating(_c: &mut Client) {
    // TODO: Save floating geometry
}

pub fn restore_floating(_c: &mut Client) {
    // TODO: Restore floating geometry
}

pub fn change_floating(_c: &mut Client) {
    // TODO: Change floating state
}
