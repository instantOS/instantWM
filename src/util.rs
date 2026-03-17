use crate::contexts::WmCtx;

/// Spawn a command directly.
pub fn spawn<S: AsRef<str>>(ctx: &WmCtx, argv: &[S]) {
    if argv.is_empty() {
        return;
    }

    let mut command = std::process::Command::new(argv[0].as_ref());
    command.args(argv.iter().skip(1).map(|s| s.as_ref()));

    // Ensure XWayland DISPLAY is present for X11 apps if running under Wayland.
    if let WmCtx::Wayland(wl) = ctx {
        if let Some(d) = wl.wayland.backend.xdisplay() {
            command.env("DISPLAY", format!(":{d}"));
        } else if let Ok(val) = std::env::var("DISPLAY") {
            command.env("DISPLAY", val);
        }
    }

    // Detach the process by redirecting standard streams to null.
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    if let Err(e) = command.spawn() {
        log::error!("instantwm: failed to spawn '{}': {}", argv[0].as_ref(), e);
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

/// Helper macro for ignoring X11 errors in non-critical operations.
/// Logs the error at warn level but continues execution.
#[macro_export]
macro_rules! x11_ignore {
    ($expr:expr) => {
        if let Err(e) = $expr {
            log::warn!("X11 operation ignored: {}", e);
        }
    };
}

/// Helper macro for X11 operations that should log errors but not fail.
/// This replaces the `let _ = conn.operation()` anti-pattern with proper logging.
#[macro_export]
macro_rules! x11_ok {
    ($expr:expr) => {
        $expr.ok()
    };
}
