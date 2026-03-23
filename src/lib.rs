#![allow(dead_code, improper_ctypes)]

// SAFETY: instantWM is single-threaded — all window manager state is confined to
// the main event loop thread. The FFI wrappers around Xlib/Xft types (Drw, Fnt,
// Color, Cursor, XlibDisplay) are only accessed from this thread, so Send+Sync
// is sound for these types.
pub mod animation;
pub mod backend;
pub mod bar;
pub mod client;
pub mod config;
pub mod constants;
pub mod contexts;
pub mod frame_clock;

pub mod floating;
pub mod focus;
pub mod globals;
pub mod ipc;
pub mod ipc_types;
pub mod keyboard;
pub mod keyboard_layout;
pub mod layouts;
pub mod monitor;
pub mod mouse;
pub mod reload;
pub mod runtime;
pub mod startup;
pub mod systray;
pub mod tags;
pub mod toggles;
pub mod types;
pub mod util;
pub mod wayland;
pub mod wm;
