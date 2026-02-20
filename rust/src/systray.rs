use crate::types::*;

pub fn update_systray() {}

pub fn get_systray_width() -> u32 {
    0
}

pub fn systray_to_mon(_m: &MonitorInner) -> Option<MonitorInner> {
    None
}

pub fn remove_systray_icon(_i: &mut ClientInner) {}

pub fn update_systray_icon_geom(_i: &mut ClientInner, _w: i32, _h: i32) {}

pub fn update_systray_icon_state(
    _i: &mut ClientInner,
    _ev: &x11rb::protocol::xproto::PropertyNotifyEvent,
) {
}

pub fn win_to_systray_icon(_w: u32) -> Option<ClientInner> {
    None
}
