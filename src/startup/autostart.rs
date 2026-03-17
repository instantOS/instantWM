use std::env;
use std::process::Command;

pub fn run_autostart() {
    if env::var("INSTANTWM_AUTOSTART").ok().as_deref() == Some("0") {
        return;
    }

    // Check if ins exists, warn if not
    let check = Command::new("command").arg("-v").arg("ins").output();

    if let Ok(output) = check {
        if !output.status.success() {
            eprintln!("instantwm: 'ins' command not found, please install instantutils");
            return;
        }
    } else {
        eprintln!("instantwm: failed to check for 'ins' command");
        return;
    }

    // Run ins autostart in the background
    match Command::new("ins").arg("autostart").spawn() {
        Ok(_) => {}
        Err(e) => {
            eprintln!("instantwm: failed to run ins autostart: {}", e);
        }
    }
}
