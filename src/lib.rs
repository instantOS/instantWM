#![allow(improper_ctypes)]

// SAFETY: instantWM is single-threaded — all window manager state is confined to
// the main event loop thread. The FFI wrappers around Xlib/Xft types (Drw, Fnt,
// Color, Cursor, XlibDisplay) are only accessed from this thread, so Send+Sync
// is sound for these types.
mod actions;
mod animation;
mod backend;
mod bar;
mod client;
pub mod config;
mod constants;
mod contexts;
mod geometry;

mod core_state;
mod floating;
mod focus;
pub mod ipc;
pub mod ipc_types;
mod keyboard;
mod keyboard_layout;
pub mod layouts;
mod model;
mod monitor;
mod mouse;
mod overview;
pub mod reload;
mod runtime;
pub mod startup;
mod systray;
mod tags;
mod toggles;
pub mod types;
mod util;
mod wayland;
mod wm;
