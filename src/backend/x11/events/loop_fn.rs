use crate::backend::BackendOps;
use crate::backend::BackendRef;
use crate::ipc::IpcServer;
use crate::wm::Wm;
use x11rb::connection::Connection;

use super::handlers;

pub fn run(wm: &mut Wm, ipc_server: &mut Option<IpcServer>) {
    use std::os::unix::io::AsRawFd;

    // Pre-fetch the X11 connection file descriptor for poll(2).
    let x11_fd = wm
        .backend
        .x11_conn()
        .map(|(conn, _)| conn.stream().as_raw_fd())
        .unwrap_or(-1);
    let ipc_fd = ipc_server.as_ref().map(|s| s.as_raw_fd()).unwrap_or(-1);

    while wm.running {
        // ── 1. Drain all pending X11 events ─────────────────────────────
        let mut handled = false;
        loop {
            let event = wm
                .backend
                .x11_conn()
                .and_then(|(conn, _)| conn.poll_for_event().ok())
                .flatten();
            match event {
                Some(event) => {
                    dispatch_event(wm, event);
                    handled = true;
                }
                None => break,
            }
        }

        // ── 2. Process any pending IPC commands ─────────────────────────
        if let Some(server) = ipc_server.as_mut() {
            server.process_pending(wm);
        }

        if wm.g.dirty.monitor_config {
            let mut ctx = wm.ctx();
            crate::monitor::apply_monitor_config(&mut ctx);
        }

        // ── 3. Wait for new data on X11 fd and/or IPC fd ────────────────
        // Skip the wait when we just handled events — there may be more
        // events that arrived while we were dispatching.
        if !handled {
            BackendRef::from_backend(&wm.backend).flush();

            let mut fds = [
                libc::pollfd {
                    fd: x11_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: ipc_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let nfds = if ipc_fd >= 0 { 2 } else { 1 };
            // Block until data arrives (or 100ms timeout as safety net).
            unsafe {
                libc::poll(fds.as_mut_ptr(), nfds as libc::nfds_t, 100);
            }
        }
    }
}

pub fn dispatch_event(wm: &mut Wm, event: x11rb::protocol::Event) {
    let ctx = wm.ctx();
    let crate::contexts::WmCtx::X11(mut ctx) = ctx else {
        return;
    };

    match event {
        x11rb::protocol::Event::ButtonPress(e) => handlers::button_press_x11(&mut ctx, &e),
        x11rb::protocol::Event::ClientMessage(e) => handlers::client_message(&mut ctx, &e),
        x11rb::protocol::Event::ConfigureNotify(e) => handlers::configure_notify(&mut ctx, &e),
        x11rb::protocol::Event::ConfigureRequest(e) => handlers::configure_request(&mut ctx, &e),
        x11rb::protocol::Event::CreateNotify(e) => handlers::create_notify(&e),
        x11rb::protocol::Event::DestroyNotify(e) => handlers::destroy_notify(&mut ctx, &e),
        x11rb::protocol::Event::EnterNotify(e) => handlers::enter_notify(&mut ctx, &e),
        x11rb::protocol::Event::Expose(e) => handlers::expose(&mut ctx, &e),
        x11rb::protocol::Event::FocusIn(e) => handlers::focus_in(&mut ctx, &e),
        x11rb::protocol::Event::KeyPress(e) => crate::keyboard::key_press_x11(&mut ctx, &e),
        x11rb::protocol::Event::KeyRelease(e) => crate::keyboard::key_release_x11(&mut ctx, &e),
        x11rb::protocol::Event::MappingNotify(e) => handlers::mapping_notify(&mut ctx, &e),
        x11rb::protocol::Event::MapRequest(e) => handlers::map_request(&mut ctx, &e),
        x11rb::protocol::Event::MotionNotify(e) => handlers::motion_notify(&mut ctx, &e),
        x11rb::protocol::Event::PropertyNotify(e) => handlers::property_notify(&mut ctx, &e),
        x11rb::protocol::Event::ResizeRequest(e) => handlers::resize_request(&mut ctx, &e),
        x11rb::protocol::Event::UnmapNotify(e) => handlers::unmap_notify(&mut ctx, &e),
        x11rb::protocol::Event::LeaveNotify(e) => handlers::leave_notify(&mut ctx, &e),
        _ => {}
    };
}
