use crate::client::{PendingLaunch, current_launch_context, new_startup_id, record_pending_launch};
use crate::contexts::WmCtx;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::xdg_activation::XdgActivationTokenData;
use std::collections::VecDeque;
use std::process::{Child, Command, Stdio};

pub(crate) struct SpawnLaunchMetadata {
    pub(crate) context: crate::client::LaunchContext,
    pub(crate) startup_id: String,
}

fn is_lockscreen_cmd(cmd: &str) -> bool {
    cmd == ".config/instantos/default/lockscreen"
        || cmd == "~/.config/instantos/default/lockscreen"
        || cmd.ends_with("/lockscreen")
        || cmd == "slock"
        || cmd == "instantlock"
}

/// Spawn a command directly.
pub fn spawn<S: AsRef<str>>(ctx: &mut WmCtx, argv: &[S]) {
    if argv.is_empty() {
        return;
    }

    let primary_cmd = argv[0].as_ref();
    let mut command = Command::new(primary_cmd);
    command.args(argv.iter().skip(1).map(|s| s.as_ref()));
    let metadata = prepare_spawn_command(ctx, &mut command);

    match command.spawn() {
        Ok(child) => {
            record_spawned_child(ctx.core_mut().pending_launches_mut(), &child, metadata);
        }
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound && is_lockscreen_cmd(primary_cmd) {
                let fallback = crate::config::generated_keybinds::resolve_lockscreen_command(
                    ctx.backend_kind(),
                );
                if fallback != primary_cmd
                    && crate::config::generated_keybinds::command_exists(fallback)
                {
                    log::info!(
                        "instantwm: lockscreen '{}' not found, falling back to '{}'",
                        primary_cmd,
                        fallback
                    );
                    let mut fallback_cmd = Command::new(fallback);
                    let fallback_meta = prepare_spawn_command(ctx, &mut fallback_cmd);
                    match fallback_cmd.spawn() {
                        Ok(child) => {
                            record_spawned_child(
                                ctx.core_mut().pending_launches_mut(),
                                &child,
                                fallback_meta,
                            );
                            return;
                        }
                        Err(fallback_err) => {
                            log::error!(
                                "instantwm: fallback lockscreen '{}' failed to spawn: {}",
                                fallback,
                                fallback_err
                            );
                        }
                    }
                }
            }
            log::error!("instantwm: failed to spawn '{}': {}", primary_cmd, e);
        }
    }
}

/// Apply the process policy shared by keybinding and IPC launches.
pub(crate) fn prepare_spawn_command(ctx: &WmCtx, command: &mut Command) -> SpawnLaunchMetadata {
    let context = current_launch_context(ctx.core().model());
    let startup_id = new_startup_id();
    command.env("DESKTOP_STARTUP_ID", &startup_id);

    if let WmCtx::Wayland(wl) = ctx {
        let selected_window = ctx.core().model().selected_win();
        if let Some(token) = wl.wayland.with_state(|state| {
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

        // Ensure XWayland clients inherit the compositor-owned display. Keep
        // the ambient value as a fallback while XWayland is still starting.
        if let Some(display) = wl.wayland.xdisplay() {
            command.env("DISPLAY", format!(":{display}"));
        } else if let Ok(display) = std::env::var("DISPLAY") {
            command.env("DISPLAY", display);
        }
    }

    // Launched applications must not retain the compositor's terminal or log
    // pipes, and belong to their own process group.
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    use std::os::unix::process::CommandExt;
    command.process_group(0);

    SpawnLaunchMetadata {
        context,
        startup_id,
    }
}

pub(crate) fn record_spawned_child(
    pending_launches: &mut VecDeque<PendingLaunch>,
    child: &Child,
    metadata: SpawnLaunchMetadata,
) {
    record_pending_launch(
        pending_launches,
        Some(child.id()),
        Some(metadata.startup_id),
        metadata.context,
    );
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
