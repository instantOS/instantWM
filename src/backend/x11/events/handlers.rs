//! X11 backend event handlers, split by domain responsibility.
//!
//! - `pointer`: Pointer button, motion, crossing (enter/leave), and touch events.
//! - `window`: Window lifecycle, configuration requests, mapping, expose, and properties.
//! - `client_message`: EWMH client messages, desktop assignment, and state changes.
//! - `randr`: Screen and CRTC resolution/reconfiguration events.

pub mod client_message;
pub mod pointer;
pub mod randr;
pub mod window;

pub use client_message::*;
pub use pointer::*;
pub use randr::*;
pub use window::*;
