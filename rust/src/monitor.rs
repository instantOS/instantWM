use crate::types::*;

pub fn create_monitor() -> MonitorInner {
    MonitorInner::default()
}

pub fn cleanup_monitor(_mon: &mut MonitorInner) {}

pub fn update_geom() -> i32 {
    0
}

pub fn update_bar_pos(_m: &mut MonitorInner) {}

pub fn update_bars() {}

pub fn resize_bar_win(_m: &mut MonitorInner) {}

pub fn dir_to_mon(_dir: i32) -> Option<MonitorInner> {
    None
}

pub fn rect_to_mon(_x: i32, _y: i32, _w: i32, _h: i32) -> Option<MonitorInner> {
    None
}

pub fn win_to_mon(_w: u32) -> Option<MonitorInner> {
    None
}

pub fn arrange(_m: &mut MonitorInner) {}

pub fn arrange_mon(_m: &mut MonitorInner) {}

pub fn restack(_m: &mut MonitorInner) {}
