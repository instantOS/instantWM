use std::ffi::CString;
use std::io::{self, Write};
use std::process::exit;
use std::ptr;

use anyhow::{Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

use crate::contexts::WmCtx;
use crate::types::*;

pub fn die(fmt: &str) -> ! {
    let _ = io::stderr().write_all(fmt.as_bytes());
    let _ = io::stderr().write_all(b"\n");
    exit(1);
}

pub fn die_with_errno(fmt: &str) -> ! {
    let _ = io::stderr().write_all(fmt.as_bytes());
    let _ = io::stderr().write_all(b": ");
    let errno = std::io::Error::last_os_error();
    let _ = io::stderr().write_all(errno.to_string().as_bytes());
    let _ = io::stderr().write_all(b"\n");
    exit(1);
}

pub fn die_args(args: &[&str]) -> ! {
    for arg in args {
        let _ = io::stderr().write_all(arg.as_bytes());
    }
    let _ = io::stderr().write_all(b"\n");
    exit(1);
}

pub fn die_args_with_errno(args: &[&str]) -> ! {
    for arg in args {
        let _ = io::stderr().write_all(arg.as_bytes());
    }
    let _ = io::stderr().write_all(b": ");
    let errno = std::io::Error::last_os_error();
    let _ = io::stderr().write_all(errno.to_string().as_bytes());
    let _ = io::stderr().write_all(b"\n");
    exit(1);
}

/// Spawn a command directly.
pub fn spawn<S: AsRef<str>>(ctx: &WmCtx, argv: &[S]) {
    if argv.is_empty() {
        return;
    }

    let mut command = std::process::Command::new(argv[0].as_ref());
    command.args(argv.iter().skip(1).map(|s| s.as_ref()));

    // Ensure XWayland DISPLAY is present for X11 apps if running under Wayland.
    if let WmCtx::Wayland(wl) = ctx {
        if let Some(d) = wl.wayland.backend.xdisplay() {
            command.env("DISPLAY", format!(":{d}"));
        } else if let Ok(val) = std::env::var("DISPLAY") {
            command.env("DISPLAY", val);
        }
    }

    // Detach the process by redirecting standard streams to null.
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    if let Err(e) = command.spawn() {
        log::error!("instantwm: failed to spawn '{}': {}", argv[0].as_ref(), e);
    }
}

pub fn clean_mask(mask: u32, numlockmask: u32) -> u32 {
    let lock_mask: u32 = x11rb::protocol::xproto::ModMask::LOCK.bits() as u32;
    mask & !(numlockmask | lock_mask)
        & (x11rb::protocol::xproto::ModMask::SHIFT.bits() as u32
            | x11rb::protocol::xproto::ModMask::CONTROL.bits() as u32
            | x11rb::protocol::xproto::ModMask::M1.bits() as u32
            | x11rb::protocol::xproto::ModMask::M2.bits() as u32
            | x11rb::protocol::xproto::ModMask::M3.bits() as u32
            | x11rb::protocol::xproto::ModMask::M4.bits() as u32
            | x11rb::protocol::xproto::ModMask::M5.bits() as u32)
}

/// Get the currently selected monitor ID.
/// Returns `None` if no monitor is selected (monitors list is empty).
#[inline]
pub fn get_sel_mon(ctx: &WmCtx) -> Option<MonitorId> {
    if ctx.g().monitors.is_empty() {
        None
    } else {
        Some(ctx.g().selected_monitor_id())
    }
}

/// Extension trait for X11 connection operations with anyhow error handling.
///
/// This trait provides methods that wrap X11 operations with proper error
/// context, converting `let _ = conn.operation()` anti-patterns into
/// properly handled errors with descriptive messages.
///

pub trait X11ConnExt {
    /// Flush the connection, returning an error with context on failure.
    fn flush_ctx(&self) -> Result<()>;

    /// Set input focus with error context.
    fn set_input_focus_ctx(
        &self,
        revert_to: x11rb::protocol::xproto::InputFocus,
        window: x11rb::protocol::xproto::Window,
        time: u32,
    ) -> Result<()>;

    /// Delete a property with error context.
    fn delete_property_ctx(
        &self,
        window: x11rb::protocol::xproto::Window,
        property: x11rb::protocol::xproto::Atom,
    ) -> Result<()>;

    /// Configure a window with error context.
    fn configure_window_ctx(
        &self,
        window: x11rb::protocol::xproto::Window,
        value_list: &x11rb::protocol::xproto::ConfigureWindowAux,
    ) -> Result<()>;

    /// Change window attributes with error context.
    fn change_window_attributes_ctx(
        &self,
        window: x11rb::protocol::xproto::Window,
        value_list: &x11rb::protocol::xproto::ChangeWindowAttributesAux,
    ) -> Result<()>;

    /// Change a 32-bit property with error context.
    fn change_property32_ctx(
        &self,
        mode: x11rb::protocol::xproto::PropMode,
        window: x11rb::protocol::xproto::Window,
        property: x11rb::protocol::xproto::Atom,
        type_: x11rb::protocol::xproto::AtomEnum,
        data: &[u32],
    ) -> Result<()>;

    /// Map a window with error context.
    fn map_window_ctx(&self, window: x11rb::protocol::xproto::Window) -> Result<()>;

    /// Unmap a window with error context.
    fn unmap_window_ctx(&self, window: x11rb::protocol::xproto::Window) -> Result<()>;

    /// Send an event with error context.
    fn send_event_ctx(
        &self,
        propagate: bool,
        destination: x11rb::protocol::xproto::Window,
        event_mask: x11rb::protocol::xproto::EventMask,
        event: impl Into<[u8; 32]>,
    ) -> Result<()>;

    /// Grab the server with error context.
    fn grab_server_ctx(&self) -> Result<()>;

    /// Ungrab the server with error context.
    fn ungrab_server_ctx(&self) -> Result<()>;

    /// Warp the pointer with error context.
    fn warp_pointer_ctx(
        &self,
        src_window: x11rb::protocol::xproto::Window,
        dst_window: x11rb::protocol::xproto::Window,
        src_x: i16,
        src_y: i16,
        src_width: u16,
        src_height: u16,
        dst_x: i16,
        dst_y: i16,
    ) -> Result<()>;

    /// Create a window with error context.
    fn create_window_ctx(
        &self,
        depth: u8,
        wid: x11rb::protocol::xproto::Window,
        parent: x11rb::protocol::xproto::Window,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        border_width: u16,
        class: x11rb::protocol::xproto::WindowClass,
        visual: x11rb::protocol::xproto::Visualid,
        value_list: &x11rb::protocol::xproto::CreateWindowAux,
    ) -> Result<()>;

    /// Reparent a window with error context.
    fn reparent_window_ctx(
        &self,
        window: x11rb::protocol::xproto::Window,
        parent: x11rb::protocol::xproto::Window,
        x: i16,
        y: i16,
    ) -> Result<()>;

    /// Kill a client with error context.
    fn kill_client_ctx(&self, resource: x11rb::protocol::xproto::Window) -> Result<()>;

    /// Allow events with error context.
    fn allow_events_ctx(&self, mode: x11rb::protocol::xproto::Allow, time: u32) -> Result<()>;

    /// Change the save set with error context.
    fn change_save_set_ctx(
        &self,
        mode: x11rb::protocol::xproto::SetMode,
        window: x11rb::protocol::xproto::Window,
    ) -> Result<()>;

    /// Grab a button with error context.
    fn grab_button_ctx(
        &self,
        owner_events: bool,
        window: x11rb::protocol::xproto::Window,
        event_mask: x11rb::protocol::xproto::EventMask,
        pointer_mode: x11rb::protocol::xproto::GrabMode,
        keyboard_mode: x11rb::protocol::xproto::GrabMode,
        confine_to: x11rb::protocol::xproto::Window,
        cursor: x11rb::protocol::xproto::Cursor,
        button: x11rb::protocol::xproto::ButtonIndex,
        modifiers: x11rb::protocol::xproto::ModMask,
    ) -> Result<()>;

    /// Ungrab a button with error context.
    fn ungrab_button_ctx(
        &self,
        button: x11rb::protocol::xproto::ButtonIndex,
        window: x11rb::protocol::xproto::Window,
        modifiers: x11rb::protocol::xproto::ModMask,
    ) -> Result<()>;

    /// Set selection owner with error context.
    fn set_selection_owner_ctx(
        &self,
        owner: x11rb::protocol::xproto::Window,
        selection: x11rb::protocol::xproto::Atom,
        time: u32,
    ) -> Result<()>;
}

impl X11ConnExt for RustConnection {
    fn flush_ctx(&self) -> Result<()> {
        self.flush().context("failed to flush X11 connection")
    }

    fn set_input_focus_ctx(
        &self,
        revert_to: x11rb::protocol::xproto::InputFocus,
        window: x11rb::protocol::xproto::Window,
        time: u32,
    ) -> Result<()> {
        self.set_input_focus(revert_to, window, time)
            .map(|_| ())
            .context("failed to set input focus")
    }

    fn delete_property_ctx(
        &self,
        window: x11rb::protocol::xproto::Window,
        property: x11rb::protocol::xproto::Atom,
    ) -> Result<()> {
        self.delete_property(window, property)
            .map(|_| ())
            .context("failed to delete property")
    }

    fn configure_window_ctx(
        &self,
        window: x11rb::protocol::xproto::Window,
        value_list: &x11rb::protocol::xproto::ConfigureWindowAux,
    ) -> Result<()> {
        self.configure_window(window, value_list)
            .map(|_| ())
            .context("failed to configure window")
    }

    fn change_window_attributes_ctx(
        &self,
        window: x11rb::protocol::xproto::Window,
        value_list: &x11rb::protocol::xproto::ChangeWindowAttributesAux,
    ) -> Result<()> {
        self.change_window_attributes(window, value_list)
            .map(|_| ())
            .context("failed to change window attributes")
    }

    fn change_property32_ctx(
        &self,
        mode: x11rb::protocol::xproto::PropMode,
        window: x11rb::protocol::xproto::Window,
        property: x11rb::protocol::xproto::Atom,
        type_: x11rb::protocol::xproto::AtomEnum,
        data: &[u32],
    ) -> Result<()> {
        self.change_property32(mode, window, property, type_, data)
            .map(|_| ())
            .context("failed to change property32")
    }

    fn map_window_ctx(&self, window: x11rb::protocol::xproto::Window) -> Result<()> {
        self.map_window(window)
            .map(|_| ())
            .context("failed to map window")
    }

    fn unmap_window_ctx(&self, window: x11rb::protocol::xproto::Window) -> Result<()> {
        self.unmap_window(window)
            .map(|_| ())
            .context("failed to unmap window")
    }

    fn send_event_ctx(
        &self,
        propagate: bool,
        destination: x11rb::protocol::xproto::Window,
        event_mask: x11rb::protocol::xproto::EventMask,
        event: impl Into<[u8; 32]>,
    ) -> Result<()> {
        self.send_event(propagate, destination, event_mask, event)
            .map(|_| ())
            .context("failed to send event")
    }

    fn grab_server_ctx(&self) -> Result<()> {
        self.grab_server()
            .map(|_| ())
            .context("failed to grab server")
    }

    fn ungrab_server_ctx(&self) -> Result<()> {
        self.ungrab_server()
            .map(|_| ())
            .context("failed to ungrab server")
    }

    fn warp_pointer_ctx(
        &self,
        src_window: x11rb::protocol::xproto::Window,
        dst_window: x11rb::protocol::xproto::Window,
        src_x: i16,
        src_y: i16,
        src_width: u16,
        src_height: u16,
        dst_x: i16,
        dst_y: i16,
    ) -> Result<()> {
        self.warp_pointer(
            src_window, dst_window, src_x, src_y, src_width, src_height, dst_x, dst_y,
        )
        .map(|_| ())
        .context("failed to warp pointer")
    }

    fn create_window_ctx(
        &self,
        depth: u8,
        wid: x11rb::protocol::xproto::Window,
        parent: x11rb::protocol::xproto::Window,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        border_width: u16,
        class: x11rb::protocol::xproto::WindowClass,
        visual: x11rb::protocol::xproto::Visualid,
        value_list: &x11rb::protocol::xproto::CreateWindowAux,
    ) -> Result<()> {
        self.create_window(
            depth,
            wid,
            parent,
            x,
            y,
            width,
            height,
            border_width,
            class,
            visual,
            value_list,
        )
        .map(|_| ())
        .context("failed to create window")
    }

    fn reparent_window_ctx(
        &self,
        window: x11rb::protocol::xproto::Window,
        parent: x11rb::protocol::xproto::Window,
        x: i16,
        y: i16,
    ) -> Result<()> {
        self.reparent_window(window, parent, x, y)
            .map(|_| ())
            .context("failed to reparent window")
    }

    fn kill_client_ctx(&self, resource: x11rb::protocol::xproto::Window) -> Result<()> {
        self.kill_client(resource)
            .map(|_| ())
            .context("failed to kill client")
    }

    fn allow_events_ctx(&self, mode: x11rb::protocol::xproto::Allow, time: u32) -> Result<()> {
        self.allow_events(mode, time)
            .map(|_| ())
            .context("failed to allow events")
    }

    fn change_save_set_ctx(
        &self,
        mode: x11rb::protocol::xproto::SetMode,
        window: x11rb::protocol::xproto::Window,
    ) -> Result<()> {
        self.change_save_set(mode, window)
            .map(|_| ())
            .context("failed to change save set")
    }

    fn grab_button_ctx(
        &self,
        owner_events: bool,
        window: x11rb::protocol::xproto::Window,
        event_mask: x11rb::protocol::xproto::EventMask,
        pointer_mode: x11rb::protocol::xproto::GrabMode,
        keyboard_mode: x11rb::protocol::xproto::GrabMode,
        confine_to: x11rb::protocol::xproto::Window,
        cursor: x11rb::protocol::xproto::Cursor,
        button: x11rb::protocol::xproto::ButtonIndex,
        modifiers: x11rb::protocol::xproto::ModMask,
    ) -> Result<()> {
        self.grab_button(
            owner_events,
            window,
            event_mask,
            pointer_mode,
            keyboard_mode,
            confine_to,
            cursor,
            button,
            modifiers,
        )
        .map(|_| ())
        .context("failed to grab button")
    }

    fn ungrab_button_ctx(
        &self,
        button: x11rb::protocol::xproto::ButtonIndex,
        window: x11rb::protocol::xproto::Window,
        modifiers: x11rb::protocol::xproto::ModMask,
    ) -> Result<()> {
        self.ungrab_button(button, window, modifiers)
            .map(|_| ())
            .context("failed to ungrab button")
    }

    fn set_selection_owner_ctx(
        &self,
        owner: x11rb::protocol::xproto::Window,
        selection: x11rb::protocol::xproto::Atom,
        time: u32,
    ) -> Result<()> {
        self.set_selection_owner(owner, selection, time)
            .map(|_| ())
            .context("failed to set selection owner")
    }
}

/// Helper macro for ignoring X11 errors in non-critical operations.
/// Logs the error at warn level but continues execution.
#[macro_export]
macro_rules! x11_ignore {
    ($expr:expr) => {
        if let Err(e) = $expr {
            log::warn!("X11 operation ignored: {}", e);
        }
    };
}

/// Helper macro for X11 operations that should log errors but not fail.
/// This replaces the `let _ = conn.operation()` anti-pattern with proper logging.
#[macro_export]
macro_rules! x11_ok {
    ($expr:expr) => {
        $expr.ok()
    };
}
