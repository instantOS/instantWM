use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    println!("cargo:rustc-link-lib=Xinerama");
    println!("cargo:rustc-link-lib=X11");
    println!("cargo:rustc-link-lib=Xft");
    println!("cargo:rustc-link-lib=Xrender");
    println!("cargo:rustc-link-lib=fontconfig");
    println!("cargo:rustc-link-lib=freetype");

    // Generate protocol version from crate version and source hash
    let crate_version = env!("CARGO_PKG_VERSION");
    let source_hash = compute_ipc_source_hash();

    // Protocol version: crate version + first 8 chars of source hash
    let protocol_version = format!(
        "{}-{}",
        crate_version,
        &source_hash[..8.min(source_hash.len())]
    );

    println!("cargo:rustc-env=IPC_PROTOCOL_VERSION={}", protocol_version);
}

/// Compute a hash of all files that affect the IPC protocol.
/// This ensures any change to IPC-related code results in a different version.
fn compute_ipc_source_hash() -> String {
    let mut hasher = DefaultHasher::new();

    // Hash all files that affect IPC protocol (must match on both client and server)
    let ipc_files = ["src/ipc_types.rs", "src/ipc.rs", "src/bin/instantwmctl.rs"];

    for file in &ipc_files {
        println!("cargo:rerun-if-changed={}", file);
        if let Ok(contents) = std::fs::read_to_string(file) {
            contents.hash(&mut hasher);
        }
    }

    // Include build timestamp to ensure different builds have different versions
    // even if source hasn't changed (catches build environment differences)
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    timestamp.hash(&mut hasher);

    format!("{:016x}", hasher.finish())
}
