use std::env;
use std::io::ErrorKind;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// Process-group id of the spawned `ins autostart` tree, `0` when none was
/// spawned (or it was already torn down).
static AUTOSTART_PGID: AtomicU32 = AtomicU32::new(0);

pub fn run_autostart() {
    if env::var("INSTANTWM_AUTOSTART").ok().as_deref() == Some("0") {
        return;
    }

    // Run ins autostart in the background.
    //
    // It gets its own process group so the whole tree can be torn down when
    // the session ends. Without this, a hung autostart (e.g. a dot update
    // waiting on an unreachable git remote) outlives the WM and poisons the
    // next session's autostart locking.
    let mut command = Command::new("ins");
    command.arg("autostart");
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    match command.spawn() {
        Ok(child) => AUTOSTART_PGID.store(child.id(), Ordering::Release),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            eprintln!("instantwm: 'ins' command not found, please install ins from instantCLI");
        }
        Err(e) => {
            eprintln!("instantwm: failed to run ins autostart: {}", e);
        }
    }
}

/// Terminate the autostart process group when the WM exits.
///
/// Children must never outlive the session: a surviving autostart tree holds
/// state (locks, git remotes) that breaks the next session's startup.
/// Note that subprocesses which detached themselves into their own process
/// group (such as ins' background dot update) are intentionally unaffected.
pub fn shutdown_autostart() {
    let pgid = AUTOSTART_PGID.swap(0, Ordering::AcqRel);
    if pgid == 0 {
        return;
    }
    // Negative pid targets the whole group. SIGTERM lets well-behaved
    // children clean up; the session is ending either way.
    // SAFETY: kill(2) with a negative pid only signals the autostart group.
    unsafe {
        libc::kill(-(pgid as i32), libc::SIGTERM);
    }
}

/// Spawn a list of commands via `sh -c`, detached from the WM process.
pub fn run_exec_commands(commands: &[String]) {
    for cmd in commands {
        if cmd.trim().is_empty() {
            continue;
        }
        match Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => {}
            Err(e) => {
                eprintln!("instantwm: exec failed for '{}': {}", cmd, e);
            }
        }
    }
}
