use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    emit_git_rerun_paths();

    println!("cargo:rustc-link-lib=Xinerama");
    println!("cargo:rustc-link-lib=X11");
    println!("cargo:rustc-link-lib=Xft");
    println!("cargo:rustc-link-lib=Xrender");
    println!("cargo:rustc-link-lib=fontconfig");
    println!("cargo:rustc-link-lib=freetype");

    // Generate protocol version from crate version and source hash
    let crate_version = env!("CARGO_PKG_VERSION");
    let source_hash = compute_ipc_source_hash();

    let build_commit = git_head_commit().unwrap_or_else(|| "unknown".to_string());

    // Protocol version: crate version + first 8 chars of source hash + first 8 chars of git commit
    let protocol_version = format!(
        "{}-{}-{}",
        crate_version,
        &source_hash[..8.min(source_hash.len())],
        &build_commit[..8.min(build_commit.len())]
    );

    println!("cargo:rustc-env=IPC_PROTOCOL_VERSION={}", protocol_version);
    println!("cargo:rustc-env=INSTANTWM_BUILD_COMMIT={}", build_commit);
}

/// Compute a hash of all files that affect the IPC protocol.
/// This ensures any change to IPC-related code results in a different version.
fn compute_ipc_source_hash() -> String {
    let mut hasher = DefaultHasher::new();

    // Hash all files that affect IPC protocol (must match on both client and server)
    let ipc_files = [
        "src/ipc_types.rs",
        "src/ipc/mod.rs",
        "src/bin/instantwmctl.rs",
    ];

    for file in &ipc_files {
        println!("cargo:rerun-if-changed={}", file);
        if let Ok(contents) = std::fs::read_to_string(file) {
            contents.hash(&mut hasher);
        }
    }

    format!("{:016x}", hasher.finish())
}

/// Re-run when the checked-out commit changes without rebuilding for staged files.
fn emit_git_rerun_paths() {
    let Some(git_dir) = git_output(&["rev-parse", "--git-dir"]).map(PathBuf::from) else {
        return;
    };

    let head = git_dir.join("HEAD");
    if head.exists() {
        println!("cargo:rerun-if-changed={}", head.display());
    }

    let head_log = git_dir.join("logs/HEAD");
    if head_log.exists() {
        println!("cargo:rerun-if-changed={}", head_log.display());
    }

    if let Some(head_ref) = git_output(&["symbolic-ref", "-q", "HEAD"]) {
        let ref_path = git_dir.join(head_ref);
        if ref_path.exists() {
            println!("cargo:rerun-if-changed={}", ref_path.display());
        } else {
            let packed_refs = git_dir.join("packed-refs");
            if packed_refs.exists() {
                println!("cargo:rerun-if-changed={}", packed_refs.display());
            }
        }
    }
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn git_head_commit() -> Option<String> {
    git_output(&["rev-parse", "HEAD"])
}
