//! One replay case per process. The Python campaign runner supplies the watchdog.

use clap::Parser;
use instantwm::fuzzing::{MAX_ACTIONS, Scenario};
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Replay hostile Wayland clients without a display or GPU")]
struct Args {
    #[arg(long, default_value_t = 0)]
    seed: u64,
    #[arg(long, default_value_t = 128)]
    steps: usize,
    /// Decode --seed using the historical action format (before RawGlobal).
    #[arg(long, conflicts_with_all = ["replay", "bytes"])]
    legacy_seed: bool,
    /// Replay an exact JSON action sequence.
    #[arg(long, conflicts_with = "bytes")]
    replay: Option<PathBuf>,
    /// Replay an unmodified cargo-fuzz artifact through the same byte decoder.
    #[arg(long)]
    bytes: Option<PathBuf>,
    /// Write the exact scenario without executing it.
    #[arg(long)]
    emit: Option<PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let scenario = if let Some(path) = args.replay {
        let data = std::fs::read(path)?;
        if data.len() > 1024 * 1024 {
            return Err("scenario file exceeds 1 MiB".into());
        }
        serde_json::from_slice::<Scenario>(&data)?
    } else if let Some(path) = args.bytes {
        if std::fs::metadata(&path)?.len() > 1024 * 1024 {
            return Err("artifact file exceeds 1 MiB".into());
        }
        Scenario::from_bytes(&std::fs::read(path)?)
    } else {
        if args.steps > MAX_ACTIONS {
            return Err(format!("--steps exceeds {MAX_ACTIONS}").into());
        }
        if args.legacy_seed {
            Scenario::seeded_legacy(args.seed, args.steps)
        } else {
            Scenario::seeded(args.seed, args.steps)
        }
    };
    if scenario.actions.len() > MAX_ACTIONS {
        return Err("too many actions".into());
    }
    if let Some(path) = args.emit {
        std::fs::write(path, serde_json::to_vec_pretty(&scenario)?)?;
    } else {
        let report = instantwm::fuzzing::run(&scenario);
        println!("{}", serde_json::to_string(&report)?);
    }
    Ok(())
}
