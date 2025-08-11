pub mod compositor;
pub mod config;
pub mod error;
pub mod input;
pub mod types;
pub mod window_manager;
pub mod top_bar;
pub mod cli;

pub use compositor::InstantCompositor;
pub use config::Config;
pub use error::{Error, Result};
pub use input::InputHandler;
pub use types::*;
pub use window_manager::WindowManager;
pub use top_bar::TopBar;
pub use cli::CliHandler;

use smithay::reexports::calloop::EventLoop;
use std::sync::{Arc, Mutex};

/// Main instantWM library entry point
pub struct InstantWM {
    pub compositor: InstantCompositor,
    pub window_manager: Arc<Mutex<WindowManager>>,
    pub input_handler: InputHandler,
    pub top_bar: TopBar,
    pub config: Config,
}

impl InstantWM {
    pub fn new() -> Result<Self> {
        let config = Config::load()?;
        let compositor = InstantCompositor::new()?;
        
        // Get screen geometry from compositor
        let screen_geometry = Rectangle {
            x: 0,
            y: 0,
            width: 1920, // TODO: Get from actual output
            height: 1080,
        };
        
        let window_manager = Arc::new(Mutex::new(
            WindowManager::new(config.clone(), screen_geometry)?
        ));
        
        let input_handler = InputHandler::new(
            window_manager.clone(),
            config.clone(),
        );
        
        let top_bar = TopBar::new(
            config.clone(),
            screen_geometry.width,
            window_manager.clone(),
        );
        
        Ok(Self {
            compositor,
            window_manager,
            input_handler,
            top_bar,
            config,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        tracing::info!("Starting instantWM");
        
        // TODO: Implement main event loop
        // This will integrate with Smithay's event loop
        
        Ok(())
    }
}