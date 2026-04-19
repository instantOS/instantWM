use crate::client::{current_launch_context, new_startup_id, record_pending_launch};
use crate::contexts::WmCtx;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::xdg_activation::XdgActivationTokenData;

pub(crate) struct SpawnLaunchMetadata {
    pub(crate) context: crate::client::LaunchContext,
    pub(crate) startup_id: String,
}

/// Spawn a command directly.
pub fn spawn<S: AsRef<str>>(ctx: &mut WmCtx, argv: &[S]) {
    if argv.is_empty() {
        return;
    }

    let mut command = std::process::Command::new(argv[0].as_ref());
    command.args(argv.iter().skip(1).map(|s| s.as_ref()));
    let metadata = configure_spawn_command(ctx, &mut command);

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

    match command.spawn() {
        Ok(child) => {
            record_pending_launch(
                ctx.core_mut().globals_mut(),
                Some(child.id()),
                Some(metadata.startup_id),
                metadata.context,
            );
        }
        Err(e) => {
            log::error!("instantwm: failed to spawn '{}': {}", argv[0].as_ref(), e);
        }
    }
}

pub(crate) fn configure_spawn_command(
    ctx: &WmCtx,
    command: &mut std::process::Command,
) -> SpawnLaunchMetadata {
    let context = current_launch_context(ctx.core().globals());
    let startup_id = new_startup_id();
    command.env("DESKTOP_STARTUP_ID", &startup_id);

    if let WmCtx::Wayland(wl) = ctx {
        let selected_window = ctx.core().selected_client();
        if let Some(token) = wl.wayland.backend.with_state(|state| {
            let source_surface = selected_window.and_then(|win| {
                state
                    .find_window(win)
                    .and_then(|window| window.wl_surface().map(|surface| surface.into_owned()))
            });
            let token_data = XdgActivationTokenData {
                surface: source_surface,
                ..Default::default()
            };
            let _ = token_data
                .user_data
                .insert_if_missing_threadsafe(|| context);
            let (token, _) = state
                .xdg_activation_state
                .create_external_token(Some(token_data));
            token.as_str().to_owned()
        }) {
            command.env("XDG_ACTIVATION_TOKEN", token);
        }
    }

    SpawnLaunchMetadata {
        context,
        startup_id,
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
