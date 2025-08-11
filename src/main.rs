use instantwm_rs::{Config, InstantWM, Result};
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(tracing_subscriber::EnvFilter::from_default_env()),
        )
        .init();

    info!("Starting instantWM");

    // Check if running as CLI
    if std::env::args().len() > 1 {
        return instantwm_rs::cli::run_cli().map_err(|e| e.into());
    }

    // Initialize instantWM
    let mut instantwm = InstantWM::new()?;
    
    // Set up signal handlers
    let _ = ctrlc::set_handler(|| {
        info!("Received interrupt signal, shutting down...");
        std::process::exit(0);
    });

    // Run the compositor
    instantwm.run()?;

    Ok(())
}