use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=.git/HEAD");

    println!("cargo:rustc-link-lib=Xinerama");
    println!("cargo:rustc-link-lib=X11");
    println!("cargo:rustc-link-lib=Xft");
    println!("cargo:rustc-link-lib=Xrender");
    println!("cargo:rustc-link-lib=fontconfig");
    println!("cargo:rustc-link-lib=freetype");

    // Generate protocol version from crate version and git commit
    let crate_version = env!("CARGO_PKG_VERSION");
    let git_hash = get_git_hash();

    // Protocol version: crate version + first 8 chars of git hash
    let protocol_version = format!("{}-{}", crate_version, &git_hash[..8.min(git_hash.len())]);

    println!("cargo:rustc-env=IPC_PROTOCOL_VERSION={}", protocol_version);
}

fn get_git_hash() -> String {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
        .trim()
        .to_string()
}
