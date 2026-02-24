use std::ffi::CString;
use std::io::{self, Write};
use std::process::exit;
use std::ptr;

use crate::config::commands::Cmd;
use crate::contexts::WmCtx;
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
pub fn spawn(ctx: &WmCtx, cmd: Cmd) {
    let argv = ctx.g.cfg.external_commands.get(cmd);
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

/// Get the currently selected monitor ID.
/// Returns `None` if no monitor is selected (monitors list is empty).
#[inline]
pub fn get_sel_mon(ctx: &WmCtx) -> Option<MonitorId> {
    if ctx.g.monitors.is_empty() {
        None
    } else {
        Some(ctx.g.selmon)
    }
}
