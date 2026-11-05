use std::collections::HashSet;
use std::fs::File;
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay::reexports::wayland_server::Resource;
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};

use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};
use crate::fuzzing::{Action, CLIENTS, Op, PEER_TAGS, Report, Scenario, WINDOWS};
use crate::types::{Monitor, Rect, Tag, TagMask};

mod wire;

type EventLoop = calloop::EventLoop<'static, WaylandState>;

struct Events {
    globals: Vec<(u32, String)>,
    last_sync: u64,
    hold_acks: bool,
    pending: Vec<Option<(xdg_surface::XdgSurface, u32)>>,
    acknowledged: Vec<bool>,
}

impl Default for Events {
    fn default() -> Self {
        Self {
            globals: Vec::new(),
            last_sync: 0,
            hold_acks: false,
            pending: vec![None; WINDOWS],
            acknowledged: vec![false; WINDOWS],
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for Events {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            state.globals.push((name, interface));
        }
    }
}

impl Dispatch<wl_callback::WlCallback, u64> for Events {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        serial: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.last_sync = state.last_sync.max(*serial);
    }
}

impl Dispatch<xdg_surface::XdgSurface, usize> for Events {
    fn event(
        state: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        slot: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            if state.hold_acks {
                state.pending[*slot] = Some((surface.clone(), serial));
            } else {
                surface.ack_configure(serial);
                state.acknowledged[*slot] = true;
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Events {
    fn event(
        _: &mut Self,
        shell: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            shell.pong(serial);
        }
    }
}

delegate_noop!(Events: ignore wl_compositor::WlCompositor);
delegate_noop!(Events: ignore wl_surface::WlSurface);
delegate_noop!(Events: ignore wl_shm::WlShm);
delegate_noop!(Events: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Events: ignore wl_buffer::WlBuffer);
delegate_noop!(Events: ignore xdg_toplevel::XdgToplevel);
delegate_noop!(Events: ignore xdg_popup::XdgPopup);
delegate_noop!(Events: ignore xdg_positioner::XdgPositioner);

struct Window {
    surface: wl_surface::WlSurface,
    xdg: xdg_surface::XdgSurface,
    top: xdg_toplevel::XdgToplevel,
    buffer: wl_buffer::WlBuffer,
}

struct Peer {
    tag: &'static str,
    server_id: smithay::reexports::wayland_server::backend::ClientId,
    conn: Connection,
    queue: EventQueue<Events>,
    events: Events,
    compositor: wl_compositor::WlCompositor,
    shell: xdg_wm_base::XdgWmBase,
    shm: wl_shm::WlShm,
    windows: Vec<Option<Window>>,
    reading: bool,
    expected_error: bool,
    /// Set by ops that intentionally push this connection past protocol
    /// limits (event bursts, oversized messages). A later disconnect is a
    /// correct server response, not a harness failure, and may surface only
    /// once subsequent writes overflow the buffer, so it never resets.
    kill_ok: bool,
    dead: bool,
    sync: u64,
}

impl Peer {
    fn connect(tag: &'static str, loop_: &mut EventLoop, state: &mut WaylandState) -> Self {
        let (socket, server) = UnixStream::pair().unwrap();
        let server_id = state
            .native
            .display_handle
            .insert_client(server, Arc::new(WaylandClientState::default()))
            .unwrap()
            .id();
        let conn = Connection::from_socket(socket).unwrap();
        let mut queue = conn.new_event_queue();
        let mut events = Events::default();
        let registry = conn.display().get_registry(&queue.handle(), ());
        conn.display().sync(&queue.handle(), 1);
        conn.flush().unwrap();
        // No blocking client roundtrip: server and clients share this thread.
        for _ in 0..64 {
            server_tick(loop_, state);
            read_events(&conn, &mut queue, &mut events).unwrap();
            if events.last_sync == 1 {
                break;
            }
        }
        assert_eq!(events.last_sync, 1, "registry handshake did not complete");
        let global = |interface: &str| {
            events
                .globals
                .iter()
                .find(|(_, name)| name == interface)
                .unwrap()
                .0
        };
        let compositor = registry.bind(global("wl_compositor"), 4, &queue.handle(), ());
        let shell = registry.bind(global("xdg_wm_base"), 1, &queue.handle(), ());
        let shm = registry.bind(global("wl_shm"), 1, &queue.handle(), ());
        Self {
            tag,
            server_id,
            conn,
            queue,
            events,
            compositor,
            shell,
            shm,
            windows: (0..WINDOWS).map(|_| None).collect(),
            reading: true,
            expected_error: false,
            kill_ok: false,
            dead: false,
            sync: 1,
        }
    }

    fn service(&mut self) {
        if self.dead {
            return;
        }
        if let Err(error) = self.conn.flush() {
            if !matches!(&error, wayland_client::backend::WaylandError::Io(error) if error.kind() == std::io::ErrorKind::WouldBlock)
            {
                assert!(
                    self.expected_error || self.kill_ok,
                    "valid client write failed: {error}"
                );
                self.dead = true;
                return;
            }
        }
        if self.reading {
            if let Err(error) = read_events(&self.conn, &mut self.queue, &mut self.events) {
                assert!(
                    self.expected_error || self.kill_ok,
                    "{} unexpectedly disconnected: {error}",
                    self.tag
                );
                self.dead = true;
            }
        }
    }

    fn destroy(&mut self, slot: usize) {
        if let Some(window) = self.windows[slot].take() {
            window.top.destroy();
            window.xdg.destroy();
            window.surface.destroy();
            window.buffer.destroy();
        }
        self.events.pending[slot] = None;
        self.events.acknowledged[slot] = false;
    }

    fn create(&mut self, slot: usize) {
        self.destroy(slot);
        let qh = self.queue.handle();
        let surface = self.compositor.create_surface(&qh, ());
        let xdg = self.shell.get_xdg_surface(&surface, &qh, slot);
        let top = xdg.get_toplevel(&qh, ());
        top.set_title(format!("instantwm-hostile-{slot}"));
        top.set_app_id("instantwm-fuzz".into());
        // The pool is real shared memory. No renderer is needed for protocol
        // lifecycle tests; rendering invalid memory belongs in a subprocess.
        let fd = unsafe { libc::memfd_create(c"instantwm-fuzz".as_ptr(), libc::MFD_CLOEXEC) };
        assert!(fd >= 0);
        let file = unsafe { File::from_raw_fd(fd) };
        file.set_len(32 * 32 * 4).unwrap();
        let pool = self.shm.create_pool(file.as_fd(), 32 * 32 * 4, &qh, ());
        let buffer = pool.create_buffer(0, 32, 32, 32 * 4, wl_shm::Format::Argb8888, &qh, ());
        pool.destroy();
        surface.commit();
        self.windows[slot] = Some(Window {
            surface,
            xdg,
            top,
            buffer,
        });
    }

    fn commit(&self, slot: usize) {
        if self.events.acknowledged[slot] {
            if let Some(w) = &self.windows[slot] {
                w.surface.attach(Some(&w.buffer), 0, 0);
                w.surface.damage(0, 0, 32, 32);
                w.surface.commit();
            }
        }
    }

    fn resume_acks(&mut self) {
        self.events.hold_acks = false;
        for (slot, pending) in self.events.pending.iter_mut().enumerate() {
            if let Some((surface, serial)) = pending.take() {
                surface.ack_configure(serial);
                self.events.acknowledged[slot] = true;
            }
        }
    }

    /// Queue server-generated event traffic toward this peer without needing
    /// its cooperation. Probes outgoing backpressure: a stalled reader may
    /// legitimately be killed past the compositor's queue limit.
    fn event_burst(&mut self, kib: u16) {
        if self.dead {
            return;
        }
        // libwayland answers wl_display.sync by itself: every request queues
        // a 24-byte callback.done + display.delete_id pair for the client.
        let replies = (1 + usize::from(kib) % 2048) * 1024 / 24;
        for _ in 0..replies {
            self.conn.display().sync(&self.queue.handle(), self.sync);
            self.sync += 1;
        }
        // WouldBlock is expected once the socket or queue limit is hit; the
        // remainder stays buffered and drains during pump().
        let _ = self.conn.flush();
    }
}

/// Read only when the fd is ready. Never use blocking_dispatch/roundtrip in
/// the process being watched: that can manufacture a harness deadlock.
fn read_events(
    conn: &Connection,
    queue: &mut EventQueue<Events>,
    events: &mut Events,
) -> Result<(), String> {
    queue.dispatch_pending(events).map_err(|e| e.to_string())?;
    for _ in 0..8 {
        let Some(guard) = conn.prepare_read() else {
            queue.dispatch_pending(events).map_err(|e| e.to_string())?;
            break;
        };
        let mut fd = libc::pollfd {
            fd: conn.as_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut fd, 1, 0) };
        if ready <= 0 {
            drop(guard);
            break;
        }
        match guard.read() {
            Ok(_) => {}
            Err(wayland_client::backend::WaylandError::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                break;
            }
            Err(error) => return Err(error.to_string()),
        }
        queue.dispatch_pending(events).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn server_tick(loop_: &mut EventLoop, state: &mut WaylandState) {
    loop_.dispatch(Some(Duration::ZERO), state).unwrap();
    state.dispatch_pending_commits();
    crate::backend::wayland::runtime::dispatch::drain_command_queue(state);
    crate::runtime::process_pending_work(&mut state.ctx(), crate::runtime::TickOptions::default());
    state.sync_space();
    state.native.space.refresh();
    state.native.popups.cleanup();
    state.native.display_handle.flush_clients().unwrap();
}

struct Harness {
    loop_: EventLoop,
    state: WaylandState,
    peers: Vec<Option<Peer>>,
    witness: Peer,
    report: Report,
}

impl Harness {
    fn new() -> Self {
        let mut wm = crate::wm::WaylandWm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.config.animations.enabled = false;
        wm.core.state.model.tags.num_tags = 3;
        let mut monitor = Monitor::default();
        monitor.monitor_rect = Rect::new(0, 0, 1280, 800);
        monitor.available_rect = monitor.monitor_rect;
        monitor.bar_default_show = false;
        monitor.init_tags(&vec![Tag::default(); 3]);
        monitor.set_selected_tags(TagMask::single(1).unwrap());
        monitor.monitor_id = wm.core.state.model.monitors.allocate_id();
        wm.core.state.model.monitors.restore(vec![monitor]);
        let (mut loop_, mut state) =
            crate::backend::wayland::compositor::new_event_loop_and_state(wm);
        // `WaylandState::new` already attempted this; the return value records
        // whether libwayland supports the raised limit, so tests know whether
        // a stalled sub-limit peer is expected to survive.
        let queue_sizing =
            super::buffer::set_default_max_buffer_size(&state.native.display_handle, 1024 * 1024);
        let witness = Peer::connect("witness", &mut loop_, &mut state);
        let mut harness = Self {
            loop_,
            state,
            peers: (0..CLIENTS).map(|_| None).collect(),
            witness,
            report: Report {
                queue_sizing,
                ..Report::default()
            },
        };
        harness.witness.create(0);
        harness.pump();
        harness.witness.commit(0);
        harness.pump();
        assert_eq!(
            harness.state.wm.core.state.model.client_count(),
            1,
            "witness did not map"
        );
        harness.report.mapped_windows = 1;
        for c in 0..CLIENTS {
            let mut peer = Peer::connect(PEER_TAGS[c], &mut harness.loop_, &mut harness.state);
            peer.create(0);
            harness.peers[c] = Some(peer);
            harness.pump();
            harness.peers[c].as_ref().unwrap().commit(0);
            harness.pump();
        }
        assert_eq!(
            harness.state.wm.core.state.model.client_count(),
            CLIENTS + 1
        );
        harness.report.mapped_windows = CLIENTS + 1;
        harness
    }

    fn pump(&mut self) {
        for _ in 0..4 {
            self.witness.service();
            for peer in self.peers.iter_mut().flatten() {
                let was_dead = peer.dead;
                peer.service();
                if !was_dead && peer.dead {
                    self.report.expected_disconnects += 1;
                }
            }
            server_tick(&mut self.loop_, &mut self.state);
        }
        self.check_invariants();
    }

    fn heartbeat(&mut self) {
        let start = Instant::now();
        self.witness.sync += 1;
        let target = self.witness.sync;
        self.witness
            .conn
            .display()
            .sync(&self.witness.queue.handle(), target);
        for _ in 0..16 {
            self.pump();
            if self.witness.events.last_sync == target {
                break;
            }
        }
        assert_eq!(
            self.witness.events.last_sync, target,
            "healthy client stopped responding"
        );
        self.report.witness_roundtrips += 1;
        self.report.max_roundtrip_micros = self
            .report
            .max_roundtrip_micros
            .max(start.elapsed().as_micros());
    }

    fn check_invariants(&self) {
        let model = &self.state.wm.core.state.model;
        let mut seen = HashSet::new();
        for (_, client) in model.clients_iter_all() {
            assert!(seen.insert(client.win), "window owned by two monitors");
            assert!(
                self.state.native.window_exists(client.win),
                "model retains destroyed window"
            );
            assert!(
                client.geo.w > 0 && client.geo.h > 0,
                "invalid managed geometry"
            );
        }
        for monitor in model.monitors.iter_all() {
            if let Some(win) = monitor.selected {
                assert!(monitor.has_client(win), "stale selection");
            }
            let mut focus = HashSet::new();
            for win in monitor.focus_order() {
                assert!(monitor.has_client(*win), "stale focus order");
                assert!(focus.insert(*win), "duplicate focus order entry");
            }
            assert_eq!(
                focus.len(),
                monitor.clients().len(),
                "client missing from focus order"
            );
        }
        assert!(self.state.native.window_index.len() <= CLIENTS * WINDOWS + WINDOWS);
        assert!(self.state.native.runtime.pending_toplevels.len() <= CLIENTS * WINDOWS + WINDOWS);
    }

    fn step(&mut self, action: &Action) {
        let c = action.client as usize % CLIENTS;
        let slot = action.window as usize % WINDOWS;
        if matches!(action.op, Op::Disconnect) {
            self.peers[c] = None;
        } else if let Op::MalformedWire(variant) = action.op {
            let mut raw = wire::malformed(&mut self.state, variant);
            self.heartbeat();
            wire::check(&mut raw, &self.state);
        } else {
            if self.peers[c]
                .as_ref()
                .is_none_or(|p| p.dead || p.expected_error)
            {
                self.peers[c] = None;
                self.pump();
                self.peers[c] = Some(Peer::connect(
                    PEER_TAGS[c],
                    &mut self.loop_,
                    &mut self.state,
                ));
            }
            // Server-side actions resolve the actual protocol surface, not a
            // fabricated model ID, and use the normal shared policy funnels.
            let win = self.peers[c]
                .as_ref()
                .and_then(|p| p.windows[slot].as_ref())
                .and_then(|w| {
                    let id = w.surface.id().protocol_id();
                    self.state
                        .native
                        .window_index
                        .iter()
                        .find_map(|(win, element)| {
                            element
                                .toplevel()
                                .filter(|top| top.wl_surface().id().protocol_id() == id)
                                .filter(|top| {
                                    top.wl_surface().client().is_some_and(|client| {
                                        client.id() == self.peers[c].as_ref().unwrap().server_id
                                    })
                                })
                                .map(|_| *win)
                        })
                });
            let peer = self.peers[c].as_mut().unwrap();
            match &action.op {
                Op::Create => peer.create(slot),
                Op::Destroy => peer.destroy(slot),
                Op::Commit => peer.commit(slot),
                Op::HoldAcks(true) => peer.events.hold_acks = true,
                Op::HoldAcks(false) => peer.resume_acks(),
                Op::StopReading(stop) => peer.reading = !stop,
                Op::EventBurst(kib) => {
                    // Probing outgoing backpressure may legitimately kill the
                    // peer: even a sub-limit burst can cross the queue limit
                    // when earlier events are still queued.
                    peer.kill_ok = true;
                    peer.event_burst(*kib);
                }
                Op::Focus => {
                    if let Some(win) = win {
                        crate::focus::focus(&mut self.state.ctx(), Some(win));
                    }
                }
                Op::CloseBurst(count) => {
                    if let Some(win) = win {
                        for _ in 0..1 + *count as usize % 32 {
                            crate::client::close_client(&mut self.state.ctx(), win);
                        }
                    }
                }
                _ => {
                    if let Some(w) = &peer.windows[slot] {
                        match &action.op {
                            Op::Unmap => {
                                w.surface.attach(None, 0, 0);
                                w.surface.commit();
                                // An unmap resets the xdg configure cycle: a
                                // buffer must not be attached until a configure
                                // received afterwards has been acked. Acking any
                                // in-flight configure restores it (see smithay's
                                // pending_configures handling).
                                peer.events.acknowledged[slot] = false;
                            }
                            Op::Fullscreen(true) => w.top.set_fullscreen(None),
                            Op::Fullscreen(false) => w.top.unset_fullscreen(),
                            Op::Maximize(true) => w.top.set_maximized(),
                            Op::Maximize(false) => w.top.unset_maximized(),
                            Op::Minimize => w.top.set_minimized(),
                            Op::Parent(parent) => {
                                // Lower slot numbers impose an acyclic parent
                                // graph, so ordinary lifecycle traces stay valid.
                                let parent = parent
                                    .map(|p| p as usize % WINDOWS)
                                    .filter(|p| *p < slot)
                                    .and_then(|p| peer.windows[p].as_ref());
                                w.top.set_parent(parent.map(|p| &p.top));
                            }
                            Op::Title(length) => {
                                // wayland-backend rejects client messages over
                                // 4096 bytes; 2-byte λ at 2000 reps keeps the
                                // request sendable (4012 bytes on the wire).
                                w.top.set_title("λ".repeat(*length as usize % 2000))
                            }
                            Op::Geometry(width, height) => w.xdg.set_window_geometry(
                                0,
                                0,
                                1 + i32::from(*width % 1024),
                                1 + i32::from(*height % 1024),
                            ),
                            Op::Constraints(width, height) => {
                                w.top.set_min_size(
                                    i32::from(*width % 128),
                                    i32::from(*height % 128),
                                );
                                w.top.set_max_size(0, 0);
                            }
                            Op::FrameBurst(count) => {
                                for _ in 0..1 + *count as usize % 64 {
                                    w.surface.frame(&peer.queue.handle(), 0);
                                    w.surface.commit();
                                }
                            }
                            Op::OversizedTitle(length) => {
                                // A single legal request above the fixed
                                // 4096-byte input buffer can never parse:
                                // the server must reject this peer while the
                                // witness and other peers keep running. The
                                // disconnect may only surface once later
                                // writes overflow the buffer, so kill_ok
                                // tolerates it for the rest of the session.
                                peer.kill_ok = true;
                                w.top
                                    .set_title("Z".repeat(4096 + usize::from(*length % 2048)));
                            }
                            Op::InvalidAck => {
                                peer.expected_error = true;
                                peer.reading = true;
                                w.xdg.ack_configure(u32::MAX);
                            }
                            Op::InvalidGeometry => {
                                peer.expected_error = true;
                                peer.reading = true;
                                w.xdg.set_window_geometry(0, 0, 0, -1);
                            }
                            Op::DuplicateRole => {
                                peer.expected_error = true;
                                peer.reading = true;
                                // smithay tolerates repeated xdg_surface and
                                // same-role requests (set_role only rejects a
                                // *different* role), so claim the popup role on
                                // an already-roled toplevel surface instead.
                                let positioner =
                                    peer.shell.create_positioner(&peer.queue.handle(), ());
                                positioner.set_size(8, 8);
                                positioner.set_anchor_rect(0, 0, 8, 8);
                                let _ =
                                    w.xdg.get_popup(None, &positioner, &peer.queue.handle(), ());
                                positioner.destroy();
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        if action.dispatch {
            self.pump();
        }
        if self.peers[c]
            .as_ref()
            .is_some_and(|p| p.expected_error && !p.dead)
        {
            self.pump();
            assert!(
                self.peers[c].as_ref().unwrap().dead,
                "invalid request was not rejected"
            );
        }
        self.report.mapped_windows = self
            .report
            .mapped_windows
            .max(self.state.wm.core.state.model.client_count());
        self.report.actions += 1;
    }

    fn finish(mut self) -> Report {
        for peer in self.peers.iter_mut() {
            *peer = None;
        }
        self.pump();
        self.heartbeat();
        assert_eq!(
            self.state.wm.core.state.model.client_count(),
            1,
            "client crash leaked model windows"
        );
        assert_eq!(
            self.state.native.window_index.len(),
            1,
            "client crash leaked native windows"
        );
        assert!(
            self.state.native.runtime.pending_toplevels.is_empty(),
            "client crash leaked pending toplevels"
        );
        // Prove subsequent mapping and focus still work after hostile cleanup.
        self.witness.create(1);
        self.pump();
        self.witness.commit(1);
        self.pump();
        assert_eq!(
            self.state.wm.core.state.model.client_count(),
            2,
            "new window failed after hostile cleanup"
        );
        let id = self.witness.windows[1]
            .as_ref()
            .unwrap()
            .surface
            .id()
            .protocol_id();
        let win = self
            .state
            .native
            .window_index
            .iter()
            .find_map(|(win, window)| {
                window
                    .toplevel()
                    .filter(|top| top.wl_surface().id().protocol_id() == id)
                    .map(|_| *win)
            })
            .expect("recovery window has no native tracking");
        crate::focus::focus(&mut self.state.ctx(), Some(win));
        assert_eq!(
            self.state.wm.core.state.model.selected_win(),
            Some(win),
            "recovery focus failed"
        );
        assert!(
            self.state
                .native
                .seat
                .get_keyboard()
                .unwrap()
                .current_focus()
                .is_some(),
            "seat focus was lost"
        );
        self.heartbeat();
        self.report
    }
}

pub(crate) fn run(scenario: &Scenario) -> Report {
    let mut harness = Harness::new();
    for (index, action) in scenario.actions.iter().enumerate() {
        harness.step(action);
        if index % 8 == 0 {
            harness.heartbeat();
        }
    }
    harness.finish()
}
