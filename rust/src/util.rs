use std::ffi::CString;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::process::exit;
use std::ptr;

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

pub fn ecalloc<T: Default + Clone>(nmemb: usize) -> Vec<T> {
    vec![T::default(); nmemb]
}

pub fn ecalloc_box<T: Default>(nmemb: usize) -> Box<[T]> {
    let mut v: Vec<T> = Vec::with_capacity(nmemb);
    for _ in 0..nmemb {
        v.push(T::default());
    }
    v.into_boxed_slice()
}

pub fn startswith(a: &str, b: &str) -> bool {
    a.starts_with(b)
}

pub fn startswith_bytes(a: &[u8], b: &[u8]) -> bool {
    a.starts_with(b)
}

pub fn spawn(arg: &Arg) {
    if let Some(v) = arg.v {
        let cmd_ptr = v as *const &str;
        let cmd = unsafe { &*cmd_ptr };
        spawn_with_args(&[*cmd], None);
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

#[inline]
pub fn min<T: Ord>(a: T, b: T) -> T {
    a.min(b)
}

#[inline]
pub fn max<T: Ord>(a: T, b: T) -> T {
    a.max(b)
}

#[inline]
pub fn between<T: Ord>(x: T, a: T, b: T) -> bool {
    a <= x && x <= b
}

#[inline]
pub fn clamp<T: Ord>(val: T, min_val: T, max_val: T) -> T {
    max(min_val, min(max_val, val))
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

pub fn length<T>(slice: &[T]) -> usize {
    slice.len()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_min() {
        assert_eq!(min(1, 2), 1);
        assert_eq!(min(2, 1), 1);
        assert_eq!(min(5, 5), 5);
    }

    #[test]
    fn test_max() {
        assert_eq!(max(1, 2), 2);
        assert_eq!(max(2, 1), 2);
        assert_eq!(max(5, 5), 5);
    }

    #[test]
    fn test_between() {
        assert!(between(5, 1, 10));
        assert!(between(1, 1, 10));
        assert!(between(10, 1, 10));
        assert!(!between(0, 1, 10));
        assert!(!between(11, 1, 10));
    }

    #[test]
    fn test_clamp() {
        assert_eq!(clamp(5, 1, 10), 5);
        assert_eq!(clamp(0, 1, 10), 1);
        assert_eq!(clamp(15, 1, 10), 10);
    }

    #[test]
    fn test_startswith() {
        assert!(startswith("hello world", "hello"));
        assert!(startswith("hello", "hello"));
        assert!(!startswith("hello", "world"));
        assert!(startswith("hello", ""));
        assert!(startswith("", ""));
    }

    #[test]
    fn test_startswith_bytes() {
        assert!(startswith_bytes(b"hello world", b"hello"));
        assert!(!startswith_bytes(b"hello", b"world"));
        assert!(startswith_bytes(b"hello", b""));
    }

    #[test]
    fn test_tagmask() {
        assert_eq!(tagmask(1), 1);
        assert_eq!(tagmask(2), 3);
        assert_eq!(tagmask(3), 7);
        assert_eq!(tagmask(9), 511);
    }
}
