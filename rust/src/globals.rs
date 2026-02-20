use crate::types::*;
use once_cell::sync::Lazy;
use std::sync::Mutex;
use x11rb::protocol::xproto::Window;

// TODO: Port global state from globals.h and instantwm.c

pub struct Globals {
    pub dpy: Option<x11rb::rust_connection::RustConnection>,
    pub screen: i32,
    pub root: Window,
    pub sw: i32,
    pub sh: i32,
    pub mons: Option<Box<Monitor>>,
    pub selmon: Option<*mut Monitor>,
    pub bh: i32,
    pub lrpad: i32,
    pub animated: bool,
    pub focusfollowsmouse: bool,
    pub focusfollowsfloatmouse: bool,
    pub altcursor: AltCursor,
    pub doubledraw: bool,
    pub specialnext: SpecialNext,
    pub bar_dragging: bool,
    pub tagwidth: i32,
    pub statuswidth: i32,
    pub showalttag: bool,
    pub tagprefix: bool,
    pub stext: [u8; 1024],
    pub wmatom: [u32; 4],
    pub netatom: [u32; 14],
    pub xatom: [u32; 3],
    pub motifatom: u32,
    pub numlockmask: u32,
    pub showsystray: bool,
}

impl Default for Globals {
    fn default() -> Self {
        Self {
            dpy: None,
            screen: 0,
            root: 0,
            sw: 0,
            sh: 0,
            mons: None,
            selmon: None,
            bh: 0,
            lrpad: 0,
            animated: true,
            focusfollowsmouse: true,
            focusfollowsfloatmouse: true,
            altcursor: AltCursor::None,
            doubledraw: false,
            specialnext: SpecialNext::None,
            bar_dragging: false,
            tagwidth: 0,
            statuswidth: 0,
            showalttag: false,
            tagprefix: false,
            stext: [0; 1024],
            wmatom: [0; 4],
            netatom: [0; 14],
            xatom: [0; 3],
            motifatom: 0,
            numlockmask: 0,
            showsystray: true,
        }
    }
}

pub static GLOBALS: Lazy<Mutex<Globals>> = Lazy::new(|| Mutex::new(Globals::default()));

pub fn get_globals() -> std::sync::MutexGuard<'static, Globals> {
    GLOBALS.lock().unwrap()
}
