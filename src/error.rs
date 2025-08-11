use thiserror::Error;

#[derive(Error, Debug)]
pub enum InstantError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("TOML parsing error: {0}")]
    Toml(#[from] toml::de::Error),
    
    #[error("Smithay error: {0}")]
    Smithay(String),
    
    #[error("Configuration error: {0}")]
    Config(String),
    
    #[error("IPC error: {0}")]
    Ipc(String),
    
    #[error("Theme error: {0}")]
    Theme(String),
}

pub type Result<T> = std::result::Result<T, InstantError>;