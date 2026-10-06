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
//!
//! # Call flow for a mouse press
//!
//! A press cannot follow that path directly. The tool is still idle when the
//! triggering click's release reaches it, and both tools read such a release as
//! a cancellation, so the press is captured and the tool starts one tick later:
//!
//! ```text
//! press (bar resize handle)
//!   └─► arm_region_selection_press  (capture owns the press, window pinned)
//!         └─► release → finish_region_selection_press (capture dropped)
//!               └─► drain_region_selection (shared tick)
//!                     └─► spawn_region_selection
//! ```

use std::io::Read;
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use crate::contexts::WmCtx;
use crate::core_state::DeferredRegionSelection;
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
///
/// This is the immediate form, for triggers that own no button press (key
/// chords, IPC). A mouse press must use
/// [`arm_region_selection_press`] instead.
pub fn draw_window(ctx: &mut WmCtx) {
    // Fail fast when nothing can receive the result; the tool itself decides
    // which monitor the rectangle lands on via its own overlays.
    let Some(win) = ctx.model().selected_win() else {
        return;
    };
    spawn_region_selection(ctx.backend_kind(), win);
}

/// Let a *mouse* press start a region selection for `window`.
///
/// The press itself is captured and the tool is spawned from
/// [`finish_region_selection_press`] on release, because both region-selection
/// tools read a button release that arrives before their own press as
/// "cancel". Spawning from the press would hand them the release of the click
/// that started them. `window` is pinned here rather than read from the
/// selection at release time, matching [`draw_window`].
///
/// Returns `false` when `window` is unmanaged, or when another interaction
/// already owns the pointer; in both cases nothing is spawned and nothing is
/// left armed.
pub fn arm_region_selection_press(
    ctx: &mut WmCtx,
    window: WindowId,
    button: MouseButton,
    source: InteractionSource,
) -> bool {
    if ctx.model().client(window).is_none() {
        return false;
    }
    ctx.transition_pointer_interaction(|drag| drag.arm_region_selection(window, button, source))
        .is_ok()
}

/// End an armed region selection and queue the tool for the next tick.
///
/// The spawn is deliberately not done here. Backends still own pointer
/// transport while the end event is being dispatched — X11 in particular
/// releases its interaction grab afterwards — and the tool grabs the pointer
/// itself, so it must not be started until that ownership is gone. Both
/// backends run [`drain_region_selection`] once per tick, after the release
/// has been fully handled.
pub fn finish_region_selection_press(ctx: &mut WmCtx, button: MouseButton) -> bool {
    let Some(armed) =
        ctx.transition_pointer_interaction(|drag| drag.finish::<DeferredRegionSelection>(button))
    else {
        return false;
    };
    ctx.pending_work_mut().queue_region_selection(armed.window);
    true
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
fn watch_region_selection(slot: &Mutex<ActiveSelection>, generation: u64) -> Option<Rect> {
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
        return None;
    }

    parse_slop_output(&output)
}

/// Start every selection whose press has ended, then apply every finished
/// selection, in completion order.
///
/// Each outcome carries the window pinned at trigger time, so a completed
/// rectangle is never discarded by a later cancellation nor applied to
/// whatever happens to be selected when the tool exits. Validation,
/// monitor migration, and the resize itself run the same funnel as the
/// historical synchronous path.
///
/// Returns `true` when at least one selection was applied this call. Starting a
/// tool is not a state change, so it is not reported.
pub fn drain_region_selection(ctx: &mut WmCtx<'_>) -> bool {
    start_pending_region_selection(ctx);

    let runtime = region_selection_runtime();
    let Ok(receiver) = runtime.receiver.lock() else {
        return false;
    };

    let mut applied = false;
    while let Ok(outcome) = receiver.try_recv() {
        let Some(rect) = outcome.rect else {
            continue;
        };
        if !is_valid_window_size(ctx.model(), &rect, outcome.window) {
            continue;
        }
        handle_monitor_switch(ctx, outcome.window, &rect);
        apply_window_resize(ctx, outcome.window, &rect);
        applied = true;
    }
    applied
}

/// Spawn the tool for a press that has already been released.
///
/// Runs from the shared tick so pointer ownership is already back with the
/// server.
fn start_pending_region_selection(ctx: &mut WmCtx<'_>) {
    let Some(win) = ctx.pending_work_mut().take_region_selection() else {
        return;
    };
    if ctx.model().client(win).is_none() {
        log::debug!("dropping region selection for closed window {win:?}");
        return;
    }
    spawn_region_selection(ctx.backend_kind(), win);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestWm as Wm;

    use crate::test_support::{MonitorBuilder, add_client_with};
    use crate::types::ClientMode;

    /// Build a window manager whose single output covers `rect`.
    fn wm_with_monitor(rect: Rect) -> (Wm, crate::types::MonitorId) {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.derived.display.width = rect.w.max(1);
        wm.core.state.derived.display.height = rect.h.max(1);
        let monitor_id = wm
            .core
            .state
            .model
            .monitors
            .push(MonitorBuilder::new().monitor_rect(rect).build());
        wm.core.state.model.monitors.set_selected(monitor_id);
        (wm, monitor_id)
    }

    fn insert_floating_client(
        ctx: &mut WmCtx<'_>,
        monitor_id: crate::types::MonitorId,
        win: WindowId,
        geo: Rect,
    ) {
        add_client_with(ctx.model_mut(), monitor_id, |client| {
            client.win = win;
            client.geo = geo;
            client.mode = ClientMode::floating();
            client.set_placement(crate::types::ClientPlacement::Floating);
        });
    }

    /// The window whose selection the next tick would start a tool for.
    fn pending_region_selection(wm: &Wm) -> Option<WindowId> {
        wm.core.work.region_selection()
    }

    fn is_tool_running() -> bool {
        let runtime = region_selection_runtime();
        let mut active = runtime.active.lock().unwrap();
        match active.child.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
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
    fn failed_selector_discards_formatted_stdout() {
        let child = std::process::Command::new("sh")
            .args(["-c", "printf x100x200x800x600x; exit 1"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let slot = Mutex::new(ActiveSelection {
            generation: 1,
            child: Some(child),
        });

        assert_eq!(watch_region_selection(&slot, 1), None);
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
        let (mut wm, monitor_id) = wm_with_monitor(Rect::new(-1920, -50, 1920, 1080));
        let win = WindowId(1);
        wm.with_ctx(|wm| {
            insert_floating_client(wm, monitor_id, win, Rect::new(-1920, -50, 600, 400))
        });

        // On the left output, well inside the layout bounds.
        assert!(is_valid_window_size(
            &wm.core.state.model,
            &Rect::new(-1900, -30, 1200, 900),
            win
        ));
        // Beyond the slop margin outside the layout origin.
        assert!(!is_valid_window_size(
            &wm.core.state.model,
            &Rect::new(-1980, -30, 1200, 900),
            win
        ));
    }

    #[test]
    fn drain_applies_outcomes_to_their_pinned_windows() {
        let (mut wm, monitor_id) = wm_with_monitor(Rect::new(0, 0, 1920, 1080));
        let pinned = WindowId(1);
        let selected = WindowId(2);
        wm.with_ctx(|wm| {
            insert_floating_client(wm, monitor_id, pinned, Rect::new(10, 10, 600, 400))
        });
        wm.with_ctx(|wm| {
            insert_floating_client(wm, monitor_id, selected, Rect::new(10, 10, 600, 400))
        });
        wm.core
            .state
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

        assert!(wm.with_ctx(drain_region_selection));
        assert_eq!(
            wm.core.state.model.client(pinned).unwrap().geo,
            Rect::new(100, 100, 1200, 900)
        );
        assert_eq!(
            wm.core.state.model.client(selected).unwrap().geo,
            Rect::new(10, 10, 600, 400)
        );
    }

    #[test]
    fn a_press_is_captured_and_spawns_nothing_until_it_is_released() {
        let (mut wm, monitor_id) = wm_with_monitor(Rect::new(0, 0, 1920, 1080));
        let win = WindowId(1);
        wm.with_ctx(|wm| insert_floating_client(wm, monitor_id, win, Rect::new(10, 10, 600, 400)));

        assert!(arm_region_selection_press(
            &mut wm.test_ctx(),
            win,
            MouseButton::Left,
            InteractionSource::Pointer,
        ));

        // The press owns the pointer but no tool exists yet, so the release
        // that is about to arrive cannot be mistaken for a cancellation.
        assert_eq!(
            wm.core.state.interaction.drag.captured_button(),
            Some(MouseButton::Left)
        );
        assert!(!is_tool_running());
        assert_eq!(pending_region_selection(&wm), None);

        assert!(finish_region_selection_press(
            &mut wm.test_ctx(),
            MouseButton::Left
        ));

        assert!(wm.core.state.interaction.drag.capture().is_none());
        assert_eq!(pending_region_selection(&wm), Some(win));
    }

    #[test]
    fn the_release_of_another_button_does_not_start_the_selection() {
        let (mut wm, monitor_id) = wm_with_monitor(Rect::new(0, 0, 1920, 1080));
        let win = WindowId(1);
        wm.with_ctx(|wm| insert_floating_client(wm, monitor_id, win, Rect::new(10, 10, 600, 400)));
        assert!(arm_region_selection_press(
            &mut wm.test_ctx(),
            win,
            MouseButton::Left,
            InteractionSource::Pointer,
        ));

        assert!(!finish_region_selection_press(
            &mut wm.test_ctx(),
            MouseButton::Right
        ));
        assert_eq!(pending_region_selection(&wm), None);
        // The press is still armed, so its own release still completes it.
        assert!(finish_region_selection_press(
            &mut wm.test_ctx(),
            MouseButton::Left
        ));
        assert_eq!(pending_region_selection(&wm), Some(win));
    }

    #[test]
    fn a_press_for_an_unmanaged_window_arms_nothing() {
        let (mut wm, _) = wm_with_monitor(Rect::new(0, 0, 1920, 1080));

        assert!(!arm_region_selection_press(
            &mut wm.test_ctx(),
            WindowId(404),
            MouseButton::Left,
            InteractionSource::Pointer,
        ));
        assert!(wm.core.state.interaction.drag.capture().is_none());
        assert_eq!(pending_region_selection(&wm), None);
    }

    #[test]
    fn the_tick_discards_a_selection_whose_window_closed_while_the_button_was_held() {
        let (mut wm, monitor_id) = wm_with_monitor(Rect::new(0, 0, 1920, 1080));
        let win = WindowId(1);
        wm.with_ctx(|wm| insert_floating_client(wm, monitor_id, win, Rect::new(10, 10, 600, 400)));
        assert!(arm_region_selection_press(
            &mut wm.test_ctx(),
            win,
            MouseButton::Left,
            InteractionSource::Pointer,
        ));
        assert!(finish_region_selection_press(
            &mut wm.test_ctx(),
            MouseButton::Left
        ));

        wm.core.state.model.remove_client(win).unwrap();
        wm.with_ctx(drain_region_selection);

        assert_eq!(pending_region_selection(&wm), None);
        assert!(!is_tool_running());
    }
}
