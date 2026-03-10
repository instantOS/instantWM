pub fn run_autostart() {
    if std::env::var("INSTANTWM_AUTOSTART").ok().as_deref() == Some("0") {
        return;
    }
    unsafe {
        match libc::fork() {
            -1 => {
                eprintln!("instantwm: fork failed for autostart");
            }
            0 => {
                libc::setsid();

                let _ = libc::system(
                    b"command -v instantautostart || { sleep 4 && notify-send 'instantutils missing, please install instantutils!!!'; } &\0"
                        .as_ptr() as *const i8,
                );
                let _ = libc::system(b"instantautostart &\0".as_ptr() as *const i8);

                libc::_exit(0);
            }
            _ => {}
        }
    }
}
