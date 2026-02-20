use crate::types::*;

pub fn apply_rules(_c: &mut ClientInner) {}

pub fn apply_size_hints(
    _c: &mut ClientInner,
    _x: &mut i32,
    _y: &mut i32,
    _w: &mut i32,
    _h: &mut i32,
    _interact: bool,
) -> bool {
    false
}

pub fn manage(_w: u32) {}

pub fn unmanage(_c: &mut ClientInner, _destroyed: bool) {}

pub fn resize(_c: &mut ClientInner, _x: i32, _y: i32, _w: i32, _h: i32, _interact: bool) {}

pub fn resize_client(_c: &mut ClientInner, _x: i32, _y: i32, _w: i32, _h: i32) {}

pub fn next_tiled(_c: Option<&ClientInner>) -> Option<&ClientInner> {
    None
}

pub fn pop(_c: &mut ClientInner) {}

pub fn set_fullscreen(_c: &mut ClientInner, _fullscreen: bool) {}

pub fn set_urgent(_c: &mut ClientInner, _urgent: bool) {}

pub fn focus(_c: Option<&mut ClientInner>) {}

pub fn unfocus(_c: &mut ClientInner, _set_focus: bool) {}

pub fn update_size_hints(_c: &mut ClientInner) {}

pub fn update_wm_hints(_c: &mut ClientInner) {}

pub fn update_motif_hints(_c: &mut ClientInner) {}

pub fn win_to_client(_w: u32) -> Option<ClientInner> {
    None
}

pub fn hide(_c: &mut ClientInner) {}

pub fn show(_c: &mut ClientInner) {}

pub fn show_hide(_c: &mut ClientInner) {}

pub fn scale_client(_c: &mut ClientInner, _scale: i32) {}

pub fn save_floating(_c: &mut ClientInner) {}

pub fn restore_floating(_c: &mut ClientInner) {}

pub fn change_floating(_c: &mut ClientInner) {}
