use crate::types::*;

pub fn button_press(_e: &x11rb::protocol::xproto::ButtonPressEvent) {}

pub fn client_message(_e: &x11rb::protocol::xproto::ClientMessageEvent) {}

pub fn configure_notify(_e: &x11rb::protocol::xproto::ConfigureNotifyEvent) {}

pub fn configure_request(_e: &x11rb::protocol::xproto::ConfigureRequestEvent) {}

pub fn destroy_notify(_e: &x11rb::protocol::xproto::DestroyNotifyEvent) {}

pub fn enter_notify(_e: &x11rb::protocol::xproto::EnterNotifyEvent) {}

pub fn expose(_e: &x11rb::protocol::xproto::ExposeEvent) {}

pub fn focus_in(_e: &x11rb::protocol::xproto::FocusInEvent) {}

pub fn key_press(_e: &x11rb::protocol::xproto::KeyPressEvent) {}

pub fn key_release(_e: &x11rb::protocol::xproto::KeyReleaseEvent) {}

pub fn mapping_notify(_e: &x11rb::protocol::xproto::MappingNotifyEvent) {}

pub fn map_request(_e: &x11rb::protocol::xproto::MapRequestEvent) {}

pub fn motion_notify(_e: &x11rb::protocol::xproto::MotionNotifyEvent) {}

pub fn property_notify(_e: &x11rb::protocol::xproto::PropertyNotifyEvent) {}

pub fn unmap_notify(_e: &x11rb::protocol::xproto::UnmapNotifyEvent) {}

pub fn resize_request(_e: &x11rb::protocol::xproto::ResizeRequestEvent) {}

pub fn run() {}

pub fn scan() {}

pub fn check_other_wm() {}

pub fn setup() {}

pub fn cleanup() {}
