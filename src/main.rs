use instantwm_rs::{Config, Result};
use smithay::{
    backend::renderer::gles::GlesRenderer,
    backend::winit::{self, WinitEvent},
    output::Mode,
    reexports::{calloop::EventLoop, wayland_server::Display},
};
use std::time::Duration;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

use instantwm_rs::compositor::InstantWMState;

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(
                    tracing_subscriber::EnvFilter::from_default_env()
                        .add_directive("instantwm_rs=debug".parse().unwrap())
                        .add_directive("smithay=warn".parse().unwrap()),
                ),
        )
        .init();

    info!(
        "Starting InstantWM Wayland Compositor v{}",
        env!("CARGO_PKG_VERSION")
    );

    // Check if running as CLI
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        return instantwm_rs::cli::run_cli()
            .map_err(|e| instantwm_rs::error::InstantError::Other(e.to_string()));
    }

    // Load configuration
    let config = Config::load()?;
    info!("Configuration loaded successfully");

    // Validate configuration
    let validation_errors = config.validate();
    if !validation_errors.is_empty() {
        warn!("Configuration validation issues:");
        for error in validation_errors {
            warn!("  - {}", error);
        }
    }

    // Create the event loop
    let mut event_loop: EventLoop<InstantWMState> = EventLoop::try_new().map_err(|e| {
        instantwm_rs::error::InstantError::Other(format!("Failed to create event loop: {}", e))
    })?;

    // Create the display
    let display = Display::new().map_err(|e| {
        instantwm_rs::error::InstantError::Other(format!("Failed to create display: {}", e))
    })?;

    // Initialize our compositor state
    let mut state = InstantWMState::new(config, display, event_loop.handle())?;

    // Set up the Wayland socket
    let socket_name = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-1".to_string());

    // For now, we'll use a simple socket name
    // In a real implementation, you'd use the display's add_socket_auto method

    info!("Wayland socket: {}", socket_name);
    state.socket_name = Some(socket_name.clone());

    // Set WAYLAND_DISPLAY environment variable
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    // Initialize winit backend for testing
    let (mut backend, mut winit) = winit::init::<GlesRenderer>().map_err(|e| {
        instantwm_rs::error::InstantError::Other(format!("Failed to initialize winit: {}", e))
    })?;

    // Create output and add to space
    let mode = Mode {
        size: (1920, 1080).into(),
        refresh: 60_000,
    };
    state
        .output
        .change_current_state(Some(mode), None, None, Some((0, 0).into()));
    state.output.set_preferred(mode);

    state.space.map_output(&state.output, (0, 0));

    // Initialize keyboard and pointer
    state
        .seat
        .add_keyboard(Default::default(), 200, 200)
        .map_err(|e| {
            instantwm_rs::error::InstantError::Other(format!(
                "Failed to initialize keyboard: {}",
                e
            ))
        })?;
    state.seat.add_pointer();

    info!("Seat initialized with keyboard and pointer");

    // Set up signal handlers for clean shutdown
    let loop_signal = event_loop.get_signal();
    ctrlc::set_handler(move || {
        info!("Received interrupt signal, shutting down...");
        loop_signal.stop();
    })
    .map_err(|e| {
        instantwm_rs::error::InstantError::Other(format!("Failed to set signal handler: {}", e))
    })?;

    // Main event loop
    info!("Starting main event loop");

    let result = event_loop.run(Duration::from_millis(16), &mut state, move |state| {
        // Handle winit events
        let _pump_status = winit.dispatch_new_events(|event| {
            match event {
                WinitEvent::Resized { size, .. } => {
                    info!("Window resized to {:?}", size);
                    // Update output mode when window is resized
                    let mode = Mode {
                        size,
                        refresh: 60_000,
                    };
                    state
                        .output
                        .change_current_state(Some(mode), None, None, Some((0, 0).into()));
                }
                WinitEvent::Input(event) => {
                    info!("Input event received");
                    // Handle input events here
                }
                WinitEvent::Redraw => {
                    // Render frame
                    info!("Redraw requested");
                }
                WinitEvent::CloseRequested => {
                    info!("Close requested");
                    // Don't stop the loop here, let the signal handler do it
                }
                _ => {}
            }
        });

        // Handle wayland events
        if let Err(e) = state.display_handle.flush_clients() {
            error!("Failed to flush clients: {}", e);
        }
    });

    match result {
        Ok(_) => info!("InstantWM shut down successfully"),
        Err(e) => error!("Error in main loop: {}", e),
    }

    Ok(())
}
