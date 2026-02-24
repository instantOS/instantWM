use std::ffi::CString;
use std::io::{self, Write};
use std::process::exit;
use std::ptr;

use x11rb::protocol::xproto::Window;

use crate::config::commands::Cmd;
use crate::globals::get_globals;
use crate::types::*;

pub fn die(fmt: &str) -> ! {
    let _ = io::stderr().write_all(fmt.as_bytes());
    let _ = io::stderr().write_all(b"\n");
    exit(1);
}

pub fn die_with_errno(fmt: &str) -> ! {
    let _ = io::stderr().write_all(fmt.as_bytes());
    let _ = io::stderr().write_all(b": ");
    let errno = std::io::Error::last_os_error();
    let _ = io::stderr().write_all(errno.to_string().as_bytes());
    let _ = io::stderr().write_all(b"\n");
    exit(1);
}

pub fn die_args(args: &[&str]) -> ! {
    for arg in args {
        let _ = io::stderr().write_all(arg.as_bytes());
    }
    let _ = io::stderr().write_all(b"\n");
    exit(1);
}

pub fn die_args_with_errno(args: &[&str]) -> ! {
    for arg in args {
        let _ = io::stderr().write_all(arg.as_bytes());
    }
    let _ = io::stderr().write_all(b": ");
    let errno = std::io::Error::last_os_error();
    let _ = io::stderr().write_all(errno.to_string().as_bytes());
    let _ = io::stderr().write_all(b"\n");
    exit(1);
}

/// Spawn a command identified by a [`Cmd`] variant.
pub fn spawn(cmd: Cmd) {
    let globals = get_globals();
    let argv = globals.external_commands.get(cmd);
    if !argv.is_empty() {
        let c_args: Vec<CString> = argv
            .iter()
            .map(|s| CString::new(*s).unwrap_or_else(|_| CString::new("").unwrap()))
            .collect();

        let args: Vec<*const libc::c_char> = c_args
            .iter()
            .map(|s| s.as_ptr())
            .chain(std::iter::once(ptr::null()))
            .collect();

        unsafe {
            match libc::fork() {
                -1 => {
                    die_with_errno("fork failed");
                }
                0 => {
                    libc::setsid();

                    libc::sigprocmask(libc::SIG_SETMASK, ptr::null(), ptr::null_mut());

                    let mut sa: libc::sigaction = std::mem::zeroed();
                    sa.sa_sigaction = libc::SIG_DFL;
                    libc::sigemptyset(&mut sa.sa_mask);
                    sa.sa_flags = 0;
                    libc::sigaction(libc::SIGCHLD, &sa, ptr::null_mut());

                    libc::execvp(c_args[0].as_ptr(), args.as_ptr());

                    die_args_with_errno(&["instantwm: execvp '", argv[0], "' failed"]);
                }
                _ => {}
            }
        }
    }
}

pub fn clean_mask(mask: u32, numlockmask: u32) -> u32 {
    let lock_mask: u32 = x11rb::protocol::xproto::ModMask::LOCK.bits() as u32;
    mask & !(numlockmask | lock_mask)
        & (x11rb::protocol::xproto::ModMask::SHIFT.bits() as u32
            | x11rb::protocol::xproto::ModMask::CONTROL.bits() as u32
            | x11rb::protocol::xproto::ModMask::M1.bits() as u32
            | x11rb::protocol::xproto::ModMask::M2.bits() as u32
            | x11rb::protocol::xproto::ModMask::M3.bits() as u32
            | x11rb::protocol::xproto::ModMask::M4.bits() as u32
            | x11rb::protocol::xproto::ModMask::M5.bits() as u32)
}

/// Get the currently selected window from the selected monitor.
/// Returns `None` if no monitor is selected or no window is selected on that monitor.
#[inline]
pub fn get_sel_win() -> Option<Window> {
    let globals = get_globals();
    globals.monitors.get(globals.selmon).and_then(|mon| mon.sel)
}

/// Get the currently selected monitor ID.
/// Returns `None` if no monitor is selected (monitors list is empty).
#[inline]
pub fn get_sel_mon() -> Option<MonitorId> {
    let globals = get_globals();
    if globals.monitors.is_empty() {
        None
    } else {
        Some(globals.selmon)
    }
}
