use crate::types::*;

pub fn draw_bar(_m: &mut MonitorInner) {}

pub fn draw_bars() {}

pub fn draw_status_bar(_m: &mut MonitorInner, _bh: i32, _text: &str) -> i32 {
    0
}

pub fn update_bar_pos(_m: &mut MonitorInner) {}

pub fn update_status() {}

pub fn get_tag_width() -> i32 {
    0
}

pub fn get_tag_at_x(_x: i32) -> i32 {
    -1
}

pub fn toggle_bar(_arg: &Arg) {}

pub fn reset_bar() {}
