use std::ffi::CString;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
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

/// Allocate a vector with default values - idiomatic Rust replacement for C's ecalloc.
/// Use `vec![T::default(); nmemb]` directly in new code.
pub fn ecalloc<T: Default + Clone>(nmemb: usize) -> Vec<T> {
    vec![T::default(); nmemb]
}

/// Allocate a boxed slice with default values.
/// Prefer using `vec![T::default(); nmemb].into_boxed_slice()` directly in new code.
pub fn ecalloc_box<T: Default + Clone>(nmemb: usize) -> Box<[T]> {
    vec![T::default(); nmemb].into_boxed_slice()
}

/// Spawn a command identified by a [`Cmd`] variant.
pub fn spawn(cmd: Cmd) {
    let globals = get_globals();
    let argv = globals.external_commands.get(cmd);
    if !argv.is_empty() {
        spawn_with_args(argv, None);
    }
}

pub fn spawn_with_args(cmd: &[&str], _extra_env: Option<&[(&str, &str)]>) {
    if cmd.is_empty() {
        return;
    }

    let c_args: Vec<CString> = cmd
        .iter()
        .map(|s| CString::new(*s).unwrap_or_else(|_| CString::new("").unwrap()))
        .collect();

    let argv: Vec<*const libc::c_char> = c_args
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

                libc::execvp(c_args[0].as_ptr(), argv.as_ptr());

                die_args_with_errno(&["instantwm: execvp '", cmd[0], "' failed"]);
            }
            _ => {}
        }
    }
}

pub fn spawn_vec(cmd: &[CString]) {
    if cmd.is_empty() {
        return;
    }

    let argv: Vec<*const libc::c_char> = cmd
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

                libc::execvp(cmd[0].as_ptr(), argv.as_ptr());

                let cmd_str = cmd[0].to_string_lossy();
                die_args_with_errno(&["instantwm: execvp '", &cmd_str, "' failed"]);
            }
            _ => {}
        }
    }
}

/// Check if a value is between two bounds (inclusive).
#[inline]
pub fn between<T: Ord>(x: T, a: T, b: T) -> bool {
    a <= x && x <= b
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

pub fn tagmask(num_tags: usize) -> u32 {
    (1 << num_tags) - 1
}

pub fn close_fd(fd: i32) -> bool {
    unsafe { libc::close(fd) == 0 }
}

pub fn get_x11_fd(conn: &x11rb::rust_connection::RustConnection) -> Option<i32> {
    Some(conn.stream().as_raw_fd())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_between() {
        assert!(between(5, 1, 10));
        assert!(between(1, 1, 10));
        assert!(between(10, 1, 10));
        assert!(!between(0, 1, 10));
        assert!(!between(11, 1, 10));
    }

    #[test]
    fn test_tagmask() {
        assert_eq!(tagmask(1), 1);
        assert_eq!(tagmask(2), 3);
        assert_eq!(tagmask(3), 7);
        assert_eq!(tagmask(9), 511);
    }
}
