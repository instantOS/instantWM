//! Winit-backend startup helpers.
//!
//! Everything that is shared with the DRM backend lives in
//! `crate::startup::common_wayland`.  This module re-exports those items and
//! adds the one winit-specific helper: `spawn_wayland_smoke_window`.

use std::process::Command;
use std::time::Duration;

// Re-export the shared helpers so that `wayland.rs` can still import them
// from `self::init` without changing its import paths.
pub(super) use crate::startup::common_wayland::sanitize_wayland_size;

/// Spawn a lightweight test window a short time after startup.
///
/// This gives the nested compositor something visible to display immediately
/// after launch during development / smoke-testing.  Set the environment
/// variable `INSTANTWM_WL_AUTOSPAWN=0` to suppress it.
pub(super) fn spawn_wayland_smoke_window() {
    if std::env::var("INSTANTWM_WL_AUTOSPAWN").ok().as_deref() == Some("0") {
        return;
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(800));
        let _ = Command::new("sh")
            .arg("-lc")
            .arg("for app in gtk3-demo thunar xmessage; do command -v \"$app\" >/dev/null 2>&1 && exec \"$app\"; done; exit 0")
            .spawn();
    });
}
