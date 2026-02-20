use crate::globals::*;
use crate::types::*;

// TODO: Port system tray from systray.c

pub fn update_systray() {
    // TODO: Update system tray
}

pub fn get_systray_width() -> u32 {
    // TODO: Get systray width
    0
}

pub fn systray_to_mon(_m: &Monitor) -> Option<Monitor> {
    // TODO: Get systray monitor
    None
}

pub fn remove_systray_icon(_i: &mut Client) {
    // TODO: Remove systray icon
}

pub fn update_systray_icon_geom(_i: &mut Client, _w: i32, _h: i32) {
    // TODO: Update systray icon geometry
}

pub fn update_systray_icon_state(
    _i: &mut Client,
    _ev: &x11rb::protocol::xproto::PropertyNotifyEvent,
) {
    // TODO: Update systray icon state
}

pub fn win_to_systray_icon(_w: u32) -> Option<Client> {
    // TODO: Find systray icon by window
    None
}
