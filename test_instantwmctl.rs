use std::process::{Command, Stdio};
use std::io::Write;

fn main() {
    let mut child = Command::new("./target/debug/instantwmctl")
        .arg("update-status")
        .arg("-")
        .env("INSTANTWM_SOCKET", "/tmp/test.sock")
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();

    let stdin = child.stdin.as_mut().unwrap();
    stdin.write_all(b"{\"version\":1}\n").unwrap();
    stdin.write_all(b"[\n").unwrap();
    stdin.write_all(b"[{\"name\":\"time\",\"instance\":\"time\",\"full_text\":\" 12:34 \"}]\n").unwrap();
    stdin.write_all(b",[{\"name\":\"time\",\"instance\":\"time\",\"full_text\":\" 12:35 \"}]\n").unwrap();
}
