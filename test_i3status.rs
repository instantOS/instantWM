use std::process::Command;

fn main() {
    let output = Command::new("i3status-rs").output().unwrap();
    println!("i3status-rs stdout:\n{}", String::from_utf8_lossy(&output.stdout));
}
