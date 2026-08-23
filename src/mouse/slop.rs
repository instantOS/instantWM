//! Region selection and geometry validation for `draw_window`.
//!
//! Interactive rectangle selection is delegated to a per-backend helper tool:
//! `instantslop` draws through the X root window, slurp renders a layer-shell
//! overlay that spans every output under Wayland. Both are spawned with
//! `-f x%xx%yx%wx%hx`, so [`parse_slop_output`] serves both.
//!
//! The tool owns overlay rendering and input capture; this process must stay
//! responsive while it runs (on Wayland the compositor is what keeps slurp's
//! surface alive). Selection therefore completes asynchronously: the watcher
//! thread delivers the outcome — the rectangle plus the window pinned at
//! trigger time — to [`drain_region_selection`], which the shared event-loop
//! tick calls.
//!
//! This module also owns the geometry-validation predicates used by external
//! callers (IPC commands, bar click handlers) that want to resize a window to
//! an arbitrary rectangle without selection.
//!
//! # Call flow for `draw_window`
//!
//! ```text
//! user triggers draw_window keybinding
//!   └─► spawn_region_selection (tool + format per backend, window pinned)
//!         └─► watcher thread: read stdout → parse → send SelectionOutcome → ping
//!               └─► drain_region_selection   (shared tick)
//!                     └─► is_valid_window_size → handle_monitor_switch
//!                           └─► apply_window_resize
//! ```

use std::io::Read;
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::mouse::monitor::handle_monitor_switch;
use crate::types::*;

use super::constants::{MIN_WINDOW_SIZE, SLOP_MARGIN};

// ── Slop output parsing ───────────────────────────────────────────────────────

/// Format string passed to the region-selection tool.
///
/// Both `instantslop -f x%xx%yx%wx%hx` and `slurp -f x%xx%yx%wx%hx` emit a
/// literal string like `x100x200x800x600x`.
pub const REGION_SELECTION_FORMAT: &str = "x%xx%yx%wx%hx";

/// Parse the output of a region-selection tool run with
/// [`REGION_SELECTION_FORMAT`] into a [`Rect`].
///
/// The leading field before the first `x` is always empty, and the four
/// values follow in the order `x`, `y`, `w`, `h`. Negative coordinates parse
/// naturally (`x-100x…`).
///
/// Returns `None` when the output is malformed or any field fails to parse as
/// an integer — including cancellation, which both tools report with empty
/// output.
pub fn parse_slop_output(output: &str) -> Option<Rect> {
    // Expected tokens after splitting on 'x': ["", x, y, w, h, ""]
    let parts: Vec<&str> = output.split('x').collect();
    if parts.len() < 5 {
        return None;
    }

    let x: i32 = parts.get(1)?.parse().ok()?;
    let y: i32 = parts.get(2)?.parse().ok()?;
    let w: i32 = parts.get(3)?.parse().ok()?;
    let h: i32 = parts.get(4)?.trim_end().parse().ok()?;

    Some(Rect { x, y, w, h })
}

// ── Geometry validation ───────────────────────────────────────────────────────

/// Return `true` when `rect` describes a rectangle that is large enough to be a
/// useful window size *and* meaningfully different from the window's current
/// geometry.
///
/// The checks performed are:
/// * `width` and `height` both exceed [`MIN_WINDOW_SIZE`].
/// * `x` and `y` are within [`SLOP_MARGIN`] pixels of the monitor-layout
///   boundary (i.e. not wildly off-screen).
/// * At least one dimension differs by more than 20 px from the current
///   geometry (prevents no-op resizes).
///
/// Selection coordinates live in the global layout space, where outputs may
/// sit left of or above the origin, so the boundary is derived from the union
/// of all monitor rectangles rather than assumed to start at (0, 0).
pub fn is_valid_window_size(model: &crate::model::WmModel, rect: &Rect, c_win: WindowId) -> bool {
    let Some(c) = model.client(c_win) else {
        return false;
    };

    let origin = monitor_layout_origin(model);

    rect.w > MIN_WINDOW_SIZE
        && rect.h > MIN_WINDOW_SIZE
        && rect.x > origin.x - SLOP_MARGIN
        && rect.y > origin.y - SLOP_MARGIN
        && ((c.geo.w - rect.w).abs() > 20
            || (c.geo.h - rect.h).abs() > 20
            || (c.geo.x - rect.x).abs() > 20
            || (c.geo.y - rect.y).abs() > 20)
}

/// Top-left corner of the bounding box of all monitors (the most negative
/// output position in the layout).
fn monitor_layout_origin(model: &crate::model::WmModel) -> crate::types::Point {
    model
        .monitors_iter_all()
        .map(|monitor| crate::types::Point::new(monitor.monitor_rect.x, monitor.monitor_rect.y))
        .fold(crate::types::Point::new(0, 0), |acc, point| {
            crate::types::Point::new(acc.x.min(point.x), acc.y.min(point.y))
        })
}

// ── Window resize helpers ─────────────────────────────────────────────────────

/// Resize `c_win` to the given rectangle, promoting it to floating first if
/// it is currently tiled.
///
/// This is the single point where all external "place this window here"
/// requests should funnel.
pub fn apply_window_resize(ctx: &mut WmCtx, c_win: WindowId, rect: &Rect) {
    let _ = crate::floating::set_window_mode(
        ctx,
        c_win,
        crate::floating::WindowModeRequest::Floating(
            crate::client::geometry::FloatingPlacementIntent::RestoreOrCenter,
        ),
    );

    ctx.move_resize(c_win, *rect, MoveResizeOptions::hinted_immediate(true));
}

// ── draw_window ───────────────────────────────────────────────────────────────

/// Let the user draw a rectangle with the backend's region-selection tool and
/// resize the focused window to it.
///
/// * X11 spawns `instantslop`; Wayland spawns `slurp`
///   ([`crate::backend::BackendKind::region_selection_command`]).
/// * The child runs asynchronously so the event loop stays responsive while
///   the user selects; the outcome lands in [`drain_region_selection`].
/// * The target window is pinned now, not when the tool exits: focus may
///   legitimately change while the overlay is up (IPC `focuswin`,
///   foreign-toplevel `Activate`), and the drawn rectangle must still apply
///   to the window the user meant — exactly what the historical synchronous
///   implementation captured by reading the selection once.
/// * Cancellation or failure changes nothing.
pub fn draw_window(ctx: &mut WmCtx) {
    // Fail fast when nothing can receive the result; the tool itself decides
    // which monitor the rectangle lands on via its own overlays.
    let Some(win) = ctx.core().model().selected_win() else {
        return;
    };
    spawn_region_selection(ctx.backend_kind(), win);
}

// ── Asynchronous selection runtime ───────────────────────────────────────────

/// One finished selection: the window the rectangle applies to (pinned when
/// the selection started) plus the drawn rectangle, if any.
struct SelectionOutcome {
    window: WindowId,
    rect: Option<Rect>,
}

/// The currently running selection. `generation` lets a superseded watcher
/// detect it no longer owns the slot; `child` is kept here so a later
/// `draw_window` press can kill a wedged tool instead of being refused.
struct ActiveSelection {
    generation: u64,
    child: Option<Child>,
}

struct RegionSelectionRuntime {
    sender: mpsc::Sender<SelectionOutcome>,
    receiver: Mutex<mpsc::Receiver<SelectionOutcome>>,
    ping: Mutex<Option<calloop::ping::Ping>>,
    active: Mutex<ActiveSelection>,
}

static REGION_SELECTION_RUNTIME: OnceLock<RegionSelectionRuntime> = OnceLock::new();

fn region_selection_runtime() -> &'static RegionSelectionRuntime {
    REGION_SELECTION_RUNTIME.get_or_init(|| {
        let (sender, receiver) = mpsc::channel();
        RegionSelectionRuntime {
            sender,
            receiver: Mutex::new(receiver),
            ping: Mutex::new(None),
            active: Mutex::new(ActiveSelection {
                generation: 0,
                child: None,
            }),
        }
    })
}

/// Register the wake ping that makes a finished selection visible to an
/// otherwise idle event loop; see `crate::runtime::make_wake_ping`.
pub fn set_region_selection_ping(ping: calloop::ping::Ping) {
    let runtime = region_selection_runtime();
    if let Ok(mut slot) = runtime.ping.lock() {
        *slot = Some(ping);
    }
}

/// Spawn the backend's region-selection tool without blocking the caller.
///
/// The tool is spawned while holding the runtime lock, which makes takeover
/// atomic: any previous selection either still runs — and is killed, its
/// watcher then reporting cancellation — or the slot is already free. A
/// wedged tool (hung overlay, stdout held open by a forked descendant) can
/// therefore never disable `draw_window` permanently; the next press simply
/// replaces it.
///
/// The watcher thread reads the tool's stdout, reaps it, and delivers the
/// outcome ([`SelectionOutcome`]) to [`drain_region_selection`], then fires
/// the registered wake ping.
///
/// Returns `false` when no tool is configured for this backend, the tool
/// could not be started, or the watcher thread could not start.
pub fn spawn_region_selection(kind: crate::backend::BackendKind, window: WindowId) -> bool {
    let Some(mut command) = kind.region_selection_command() else {
        return false;
    };

    command
        .args(["-f", REGION_SELECTION_FORMAT])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let runtime = region_selection_runtime();
    let Ok(mut active) = runtime.active.lock() else {
        return false;
    };

    let child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            log::warn!(
                "region-selection tool {:?} could not be started: {err}",
                command.get_program()
            );
            return false;
        }
    };

    // Install the new child, then reap the previous tool outside the lock:
    // kill()/wait() may block, and no thread may hold the lock across a
    // blocking reap (watch_region_selection also waits after taking the
    // child out, so the two can no longer stall on each other).
    let previous = active.child.take();
    active.child = Some(child);
    active.generation += 1;
    let generation = active.generation;
    drop(active);

    if let Some(mut previous) = previous {
        log::debug!("superseding in-flight region selection; killing previous tool");
        let _ = previous.kill();
        let _ = previous.wait();
    }

    let sender = runtime.sender.clone();
    let ping = region_selection_ping();
    let active_slot = &runtime.active;
    let watcher = std::thread::Builder::new()
        .name("instantwm-region-select".to_string())
        .spawn(move || {
            let rect = watch_region_selection(active_slot, generation);
            let _ = sender.send(SelectionOutcome { window, rect });
            if let Some(ping) = ping {
                ping.ping();
            } else {
                // Without a registered ping the shared tick still drains on
                // the next input event; trace-level so idle-loop latency is
                // diagnosable rather than mysterious.
                log::trace!("region selection finished without a wake ping registered");
            }
        });

    match watcher {
        Ok(_) => true,
        Err(err) => {
            log::warn!("spawning region-selection watcher failed: {err}");
            // The child already sits in the slot; clean it up so no
            // orphaned tool outlives its failed watcher. Taken out under a
            // short lock and reaped outside it, like the takeover path.
            let cleanup = {
                let Ok(mut active) = runtime.active.lock() else {
                    return false;
                };
                if active.generation == generation {
                    active.child.take()
                } else {
                    None
                }
            };
            if let Some(mut child) = cleanup {
                let _ = child.kill();
                let _ = child.wait();
            }
            false
        }
    }
}

fn region_selection_ping() -> Option<calloop::ping::Ping> {
    let runtime = region_selection_runtime();
    let slot = runtime.ping.lock().ok()?;
    slot.clone()
}

/// Wait for one selection tool, read its stdout, and parse the rectangle.
///
/// Called only from the watcher thread. The lock is never held across a
/// blocking call: stdout is taken out under a short lock, read to EOF
/// (which happens when the tool exits — or when a takeover kills it), and
/// the child is reaped after being taken out of the slot and released.
/// Non-zero exit status is how both tools report cancellation (Escape);
/// parsing empty output already yields `None`, the status only refines the
/// log line.
fn watch_region_selection(
    slot: &Mutex<ActiveSelection>,
    generation: u64,
) -> Option<Rect> {
    let stdout = {
        let Ok(mut active) = slot.lock() else {
            return None;
        };
        if active.generation != generation {
            // Superseded before this watcher started; the takeover already
            // killed and reaped the tool.
            return None;
        }
        active.child.as_mut()?.stdout.take()
    };

    let mut output = String::new();
    if let Some(mut stream) = stdout
        && let Err(err) = stream.read_to_string(&mut output)
    {
        log::debug!("reading region-selection output failed: {err}");
    }

    // Reap the tool — unless a newer selection has taken over, in which
    // case the takeover owns the child and this rectangle is dropped: a
    // stale rect from a superseded trigger must not resize the pinned
    // window while the newer selection still runs. The child is taken out
    // of the slot under a short lock and reaped outside it, so a blocked
    // wait() never stalls takeover or the main thread.
    let status = {
        let Ok(mut active) = slot.lock() else {
            return None;
        };
        if active.generation != generation {
            // Superseded: the takeover already killed and reaped the tool.
            return None;
        }
        let mut child = active.child.take()?;
        drop(active);
        child.wait().ok()
    };
    if let Some(status) = &status
        && !status.success()
    {
        log::debug!("region selection cancelled or failed ({status})");
    }

    parse_slop_output(&output)
}

/// Apply every finished selection, in completion order.
///
/// Each outcome carries the window pinned at trigger time, so a completed
/// rectangle is never discarded by a later cancellation nor applied to
/// whatever happens to be selected when the tool exits. Validation,
/// monitor migration, and the resize itself run the same funnel as the
/// historical synchronous path.
///
/// Returns `true` when at least one selection was applied this call.
pub fn drain_region_selection(wm: &mut crate::wm::Wm) -> bool {
    let runtime = region_selection_runtime();
    let Ok(receiver) = runtime.receiver.lock() else {
        return false;
    };

    let mut applied = false;
    while let Ok(outcome) = receiver.try_recv() {
        let Some(rect) = outcome.rect else {
            continue;
        };
        if !is_valid_window_size(&wm.core.model, &rect, outcome.window) {
            continue;
        }

        let mut ctx = wm.ctx();
        handle_monitor_switch(&mut ctx, outcome.window, &rect);
        apply_window_resize(&mut ctx, outcome.window, &rect);
        applied = true;
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::types::{Client, ClientMode, Monitor};
    use crate::wm::Wm;

    fn wm_with_monitor(monitor: Monitor) -> (Wm, crate::types::MonitorId) {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.derived.display.width = monitor.monitor_rect.w.max(1);
        wm.core.derived.display.height = monitor.monitor_rect.h.max(1);
        let monitor_id = wm.core.model.monitors.push(monitor);
        wm.core.model.monitors.set_selected(monitor_id);
        (wm, monitor_id)
    }

    fn insert_floating_client(
        wm: &mut Wm,
        monitor_id: crate::types::MonitorId,
        win: WindowId,
        geo: Rect,
    ) {
        let mut client = Client {
            win,
            monitor_id,
            geo,
            mode: ClientMode::floating(),
            ..Client::default()
        };
        client.set_placement(crate::types::ClientPlacement::Floating);
        wm.core.model.insert_client(client);
        wm.core
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .clients
            .push(win);
    }

    #[test]
    fn parses_the_shared_format_for_both_tools() {
        assert_eq!(
            parse_slop_output("x100x200x800x600x"),
            Some(Rect {
                x: 100,
                y: 200,
                w: 800,
                h: 600
            })
        );
    }

    #[test]
    fn parses_negative_origins_produced_by_outputs_left_of_the_origin() {
        assert_eq!(
            parse_slop_output("x-1920x-50x1200x900x"),
            Some(Rect {
                x: -1920,
                y: -50,
                w: 1200,
                h: 900
            })
        );
    }

    #[test]
    fn cancellation_and_garbage_yield_none() {
        assert_eq!(parse_slop_output(""), None);
        assert_eq!(parse_slop_output("cancelled\n"), None);
        assert_eq!(parse_slop_output("x10x20xbadx600x"), None);
        assert_eq!(parse_slop_output("x10x20"), None);
    }

    #[test]
    fn trailing_newline_is_trimmed_from_height() {
        assert_eq!(
            parse_slop_output("x0x0x100x80\n"),
            Some(Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 80
            })
        );
    }

    #[test]
    fn selections_on_outputs_left_of_the_origin_validate() {
        let (mut wm, monitor_id) = wm_with_monitor(Monitor {
            monitor_rect: Rect::new(-1920, -50, 1920, 1080),
            ..Monitor::default()
        });
        let win = WindowId(1);
        insert_floating_client(&mut wm, monitor_id, win, Rect::new(-1920, -50, 600, 400));

        // On the left output, well inside the layout bounds.
        assert!(is_valid_window_size(
            &wm.core.model,
            &Rect::new(-1900, -30, 1200, 900),
            win
        ));
        // Beyond the slop margin outside the layout origin.
        assert!(!is_valid_window_size(
            &wm.core.model,
            &Rect::new(-1980, -30, 1200, 900),
            win
        ));
    }

    #[test]
    fn drain_applies_outcomes_to_their_pinned_windows() {
        let (mut wm, monitor_id) = wm_with_monitor(Monitor {
            monitor_rect: Rect::new(0, 0, 1920, 1080),
            available_rect: Rect::new(0, 0, 1920, 1080),
            ..Monitor::default()
        });
        let pinned = WindowId(1);
        let selected = WindowId(2);
        insert_floating_client(&mut wm, monitor_id, pinned, Rect::new(10, 10, 600, 400));
        insert_floating_client(&mut wm, monitor_id, selected, Rect::new(10, 10, 600, 400));
        wm.core
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .set_selected(Some(selected));

        let runtime = region_selection_runtime();
        // Focus moved while the overlay was up; the rectangle still applies
        // to the window pinned at trigger time, and a queued cancellation
        // does not discard the finished rectangle in front of it.
        runtime
            .sender
            .send(SelectionOutcome {
                window: pinned,
                rect: Some(Rect::new(100, 100, 1200, 900)),
            })
            .unwrap();
        runtime
            .sender
            .send(SelectionOutcome {
                window: selected,
                rect: None,
            })
            .unwrap();

        assert!(drain_region_selection(&mut wm));
        assert_eq!(
            wm.core.model.client(pinned).unwrap().geo,
            Rect::new(100, 100, 1200, 900)
        );
        assert_eq!(
            wm.core.model.client(selected).unwrap().geo,
            Rect::new(10, 10, 600, 400)
        );
    }
}
