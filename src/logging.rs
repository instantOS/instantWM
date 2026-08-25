//! Minimal stderr logger installed at WM startup.
//!
//! The codebase logs through the `log` crate, but until now nothing installed
//! a backend, so every message was silently discarded. This fallback stays
//! dependency-free while making warnings (failed actions, dropped IPC
//! responses, backend errors) visible on stderr.
//!
//! Verbosity defaults to warnings; set `INSTANTWM_LOG=info|debug|trace` to
//! raise it (or `error`/`off` to lower it).

use log::{LevelFilter, Log, Metadata, Record};
use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};

/// Mirrors [`LevelFilter`]'s discriminants so the level can be shared with the
/// logger through a static.
static MAX_LEVEL: AtomicU8 = AtomicU8::new(LevelFilter::Warn as u8);

struct StderrLogger;

impl Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() as u8 <= MAX_LEVEL.load(Ordering::Relaxed)
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "instantwm [{}]: {}", record.level(), record.args());
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }
}

pub fn init() {
    let filter = env_level().unwrap_or(LevelFilter::Warn);
    MAX_LEVEL.store(filter as u8, Ordering::Relaxed);
    // Installing twice would panic; the first caller wins and later calls are
    // harmless no-ops.
    let _ = log::set_boxed_logger(Box::new(StderrLogger));
    log::set_max_level(filter);
}

fn env_level() -> Option<LevelFilter> {
    let raw = std::env::var_os("INSTANTWM_LOG")?;
    Some(match raw.to_string_lossy().to_ascii_lowercase().as_str() {
        "off" => LevelFilter::Off,
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => return None,
    })
}
