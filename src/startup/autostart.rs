use std::env;
use std::io::ErrorKind;
use std::process::Command;

pub fn run_autostart() {
    if env::var("INSTANTWM_AUTOSTART").ok().as_deref() == Some("0") {
        return;
    }

    // Run ins autostart in the background
    match Command::new("ins").arg("autostart").spawn() {
        Ok(_) => {}
        Err(err) if err.kind() == ErrorKind::NotFound => {
            eprintln!("instantwm: 'ins' command not found, please install ins from instantCLI");
        }
        Err(e) => {
            eprintln!("instantwm: failed to run ins autostart: {}", e);
        }
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
