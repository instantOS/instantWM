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

pub fn startswith(a: &str, b: &str) -> bool {
    a.starts_with(b)
}

/// Check if byte slice starts with another byte slice.
/// This is a wrapper around the standard library's starts_with for byte slices.
pub fn startswith_bytes(a: &[u8], b: &[u8]) -> bool {
    a.starts_with(b)
}

/// Spawn a command identified by a [`Cmd`] variant stored in `arg.v`.
///
/// The [`Cmd`] variant is cast to `usize` when building keybindings/buttons
/// (via `Cmd::Foo as usize`), then reconstructed here via
/// [`cmd_from_usize`] and resolved against the globals' [`ExternalCommands`].
pub fn spawn(arg: &Arg) {
    let id = match arg.v {
        Some(v) => v,
        None => return,
    };

    let cmd_variant = cmd_from_usize(id);
    let globals = get_globals();
    let argv = globals.external_commands.get(cmd_variant);
    if !argv.is_empty() {
        spawn_with_args(argv, None);
    }
}

/// Reconstruct a [`Cmd`] from its `usize` discriminant.
///
/// Unknown values map to [`Cmd::Default`] (a no-op) so stale bindings fail
/// gracefully instead of panicking.
pub fn cmd_from_usize(id: usize) -> Cmd {
    // Keep in sync with the `Cmd` enum discriminants in config/commands.rs.
    match id {
        x if x == Cmd::Default as usize => Cmd::Default,
        x if x == Cmd::Term as usize => Cmd::Term,
        x if x == Cmd::TermScratch as usize => Cmd::TermScratch,
        x if x == Cmd::InstantMenu as usize => Cmd::InstantMenu,
        x if x == Cmd::ClipMenu as usize => Cmd::ClipMenu,
        x if x == Cmd::Smart as usize => Cmd::Smart,
        x if x == Cmd::InstantMenuSt as usize => Cmd::InstantMenuSt,
        x if x == Cmd::QuickMenu as usize => Cmd::QuickMenu,
        x if x == Cmd::InstantAssist as usize => Cmd::InstantAssist,
        x if x == Cmd::InstantRepeat as usize => Cmd::InstantRepeat,
        x if x == Cmd::InstantPacman as usize => Cmd::InstantPacman,
        x if x == Cmd::InstantShare as usize => Cmd::InstantShare,
        x if x == Cmd::Nautilus as usize => Cmd::Nautilus,
        x if x == Cmd::Slock as usize => Cmd::Slock,
        x if x == Cmd::OneKeyLock as usize => Cmd::OneKeyLock,
        x if x == Cmd::LangSwitch as usize => Cmd::LangSwitch,
        x if x == Cmd::OsLock as usize => Cmd::OsLock,
        x if x == Cmd::Help as usize => Cmd::Help,
        x if x == Cmd::Search as usize => Cmd::Search,
        x if x == Cmd::KeyLayoutSwitch as usize => Cmd::KeyLayoutSwitch,
        x if x == Cmd::ISwitch as usize => Cmd::ISwitch,
        x if x == Cmd::InstantSwitch as usize => Cmd::InstantSwitch,
        x if x == Cmd::CaretInstantSwitch as usize => Cmd::CaretInstantSwitch,
        x if x == Cmd::InstantSkippy as usize => Cmd::InstantSkippy,
        x if x == Cmd::Onboard as usize => Cmd::Onboard,
        x if x == Cmd::InstantShutdown as usize => Cmd::InstantShutdown,
        x if x == Cmd::SystemMonitor as usize => Cmd::SystemMonitor,
        x if x == Cmd::Notify as usize => Cmd::Notify,
        x if x == Cmd::Yazi as usize => Cmd::Yazi,
        x if x == Cmd::Panther as usize => Cmd::Panther,
        x if x == Cmd::ControlCenter as usize => Cmd::ControlCenter,
        x if x == Cmd::Display as usize => Cmd::Display,
        x if x == Cmd::PavuControl as usize => Cmd::PavuControl,
        x if x == Cmd::InstantSettings as usize => Cmd::InstantSettings,
        x if x == Cmd::Code as usize => Cmd::Code,
        x if x == Cmd::StartMenu as usize => Cmd::StartMenu,
        x if x == Cmd::Scrot as usize => Cmd::Scrot,
        x if x == Cmd::FScrot as usize => Cmd::FScrot,
        x if x == Cmd::ClipScrot as usize => Cmd::ClipScrot,
        x if x == Cmd::FClipScrot as usize => Cmd::FClipScrot,
        x if x == Cmd::Firefox as usize => Cmd::Firefox,
        x if x == Cmd::Editor as usize => Cmd::Editor,
        x if x == Cmd::PlayerNext as usize => Cmd::PlayerNext,
        x if x == Cmd::PlayerPrevious as usize => Cmd::PlayerPrevious,
        x if x == Cmd::PlayerPause as usize => Cmd::PlayerPause,
        x if x == Cmd::Spoticli as usize => Cmd::Spoticli,
        x if x == Cmd::UpVol as usize => Cmd::UpVol,
        x if x == Cmd::DownVol as usize => Cmd::DownVol,
        x if x == Cmd::MuteVol as usize => Cmd::MuteVol,
        x if x == Cmd::UpBright as usize => Cmd::UpBright,
        x if x == Cmd::DownBright as usize => Cmd::DownBright,
        x if x == Cmd::Tag as usize => Cmd::Tag,
        _ => Cmd::Default,
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

/// Return the minimum of two values. Alias for `std::cmp::min`.
#[inline]
pub fn min<T: Ord>(a: T, b: T) -> T {
    a.min(b)
}

/// Return the maximum of two values. Alias for `std::cmp::max`.
#[inline]
pub fn max<T: Ord>(a: T, b: T) -> T {
    a.max(b)
}

/// Check if a value is between two bounds (inclusive).
#[inline]
pub fn between<T: Ord>(x: T, a: T, b: T) -> bool {
    a <= x && x <= b
}

/// Clamp a value between min and max bounds.
#[inline]
pub fn clamp<T: Ord>(val: T, min_val: T, max_val: T) -> T {
    val.clamp(min_val, max_val)
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
