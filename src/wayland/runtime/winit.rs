//! Wayland compositor runtime for the winit (nested) backend.
//!
//! The winit backend runs as a nested compositor inside an existing
//! Wayland or X11 session.

use std::process::exit;

use smithay::backend::input::{Event, InputEvent};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent};
use smithay::reexports::calloop::LoopSignal;

use crate::backend::wayland::commands::WmCommand;
use crate::backend::wayland::compositor::WaylandState;
use crate::monitor::refresh_monitor_layout;
use crate::wayland::common::sanitize_wayland_size;
use crate::wayland::input::{apply_pending_warp, handle_keyboard};
use crate::wayland::render::winit::render_frame;

/// Run the winit (nested) Wayland compositor.
pub fn run() -> ! {
    let mut wm = super::common::create_wayland_wm_boxed();
    let (mut event_loop, mut state) = super::common::new_wayland_event_loop_and_state();
    let loop_handle = event_loop.handle();
    state.attach_wm(&mut wm);
    super::common::attach_wayland_backend_state(&mut wm, &mut state);

    crate::runtime::init_keyboard_layout(&mut wm);

    let (backend_init, winit_loop) =
        winit::init::<GlesRenderer>().expect("failed to init winit backend");
    let mut backend = Box::new(backend_init);
    super::common::attach_gles_renderer_and_protocols(&mut state, backend.renderer(), None);

    let output_size = backend.window_size();
    let (initial_w, initial_h) = sanitize_wayland_size(output_size.w, output_size.h);
    wm.g.cfg.screen_width = initial_w;
    wm.g.cfg.screen_height = initial_h;
    refresh_monitor_layout(&mut wm.ctx());

    // Store initial window size for the calloop source callback.
    state.runtime.winit_window_size = output_size;

    let output = state.create_output("winit", initial_w, initial_h);
    crate::monitor::apply_monitor_config(&mut wm.ctx());
    let mut damage_tracker =
        smithay::backend::renderer::damage::OutputDamageTracker::from_output(&output);

    let keyboard_handle = state.keyboard.clone();
    let pointer_handle = state.pointer.clone();

    super::common::setup_wayland_listen_socket_xwayland_systray(&loop_handle, &state, &mut wm);

    let mut ipc_server = super::common::wayland_autostart_ipc_status_ping(&loop_handle, &wm);

    let (render_ping, render_ping_source) = calloop::ping::make_ping().expect("ping");
    loop_handle
        .insert_source(render_ping_source, |_, _, _| {})
        .expect("render ping source");
    state.runtime.render_ping = Some(render_ping);

    // ── Winit event source ──────────────────────────────────────────────
    // Insert the winit event loop as a calloop source so host window
    // events (input, resize, close) wake the event loop immediately
    // instead of requiring periodic polling.
    let kb = keyboard_handle.clone();
    loop_handle
        .insert_source(winit_loop, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => {
                state.runtime.winit_window_size = size;
                state.runtime.pending_winit_resize = Some((size.w, size.h));
            }
            WinitEvent::Input(event) => {
                dispatch_winit_input(state, &kb, event);
            }
            WinitEvent::CloseRequested => {
                state.runtime.winit_close_requested = true;
            }
            WinitEvent::Redraw | WinitEvent::Focus(_) => {}
        })
        .expect("failed to insert winit source");

    let start_time = std::time::Instant::now();

    crate::runtime::spawn_status_bar(&wm);

    // ── Animation timer (on-demand) ─────────────────────────────────────
    let anim_guard = crate::runtime::AnimationTimerGuard::new();
    let loop_handle_for_timer = event_loop.handle();

    let loop_signal: LoopSignal = event_loop.get_signal();
    event_loop
        .run(None, &mut state, move |state| {
            // ── 1. Process buffered winit resize/close ──────────────────
            if let Some((w, h)) = state.runtime.pending_winit_resize.take() {
                crate::wayland::input::handle_resize(&mut wm, state, &output, w, h);
            }
            if state.runtime.winit_close_requested {
                loop_signal.stop();
                return;
            }

            // ── 2. Shared tick: layout, IPC, monitor config ─────────────
            super::common::event_loop_tick(&mut wm, state, &mut ipc_server);

            // Winit has no libinput devices to reconfigure, but clear the
            // pending bit so it doesn't remain queued forever (scroll_factor is
            // already applied at the compositor level in handle_pointer_axis).
            wm.g.pending.input_config = false;

            let animation_tick = super::common::process_window_animations(state);

            // ── 3. Arm animation timer if needed ────────────────────────
            anim_guard.ensure_armed(
                state.has_active_window_animations(),
                &loop_handle_for_timer,
                |_state| {
                    // Timer wakes the loop; animation ticking + render
                    // happen in the main body on the next iteration.
                    _state.has_active_window_animations()
                },
            );

            // Apply any compositor-side cursor warp requested during this tick
            // (e.g. from a warp-to-focus keybinding or IPC command).
            apply_pending_warp(state, &pointer_handle);

            render_frame(
                &mut wm,
                state,
                &mut backend,
                &output,
                &mut damage_tracker,
                start_time,
            );

            if animation_tick.needs_redraw() {
                state.request_render();
            }

            if state.display_handle.flush_clients().is_err() {
                loop_signal.stop();
            }
        })
        .expect("wayland event loop run");
    exit(0);
}

/// Dispatch a winit input event, pushing commands for deferred processing.
fn dispatch_winit_input(
    state: &mut WaylandState,
    keyboard_handle: &smithay::input::keyboard::KeyboardHandle<WaylandState>,
    event: InputEvent<smithay::backend::winit::WinitInput>,
) {
    use smithay::backend::input::{AbsolutePositionEvent, PointerAxisEvent, PointerButtonEvent};

    state.notify_activity();
    match event {
        InputEvent::Keyboard { event } => {
            // Keyboard events need synchronous WM access for keybindings.
            // SAFETY: the calloop source callback runs synchronously within
            // event_loop.dispatch(); the &mut Wm borrow in the main body has
            // not yet resumed.
            if let Some(wm_ptr) = unsafe { state.wm_mut_ptr() } {
                let wm = unsafe { &mut *wm_ptr };
                handle_keyboard(wm, state, keyboard_handle, event);
            }
        }
        InputEvent::PointerMotionAbsolute { event: motion } => {
            let size = state.runtime.winit_window_size;
            let x = motion.x_transformed(size.w);
            let y = motion.y_transformed(size.h);
            state.runtime.pointer_location = smithay::utils::Point::from((x, y));
            state.push_command(WmCommand::PointerMotion {
                time_msec: motion.time_msec(),
            });
        }
        InputEvent::PointerButton { event: btn } => {
            state.push_command(WmCommand::PointerButton {
                button: btn.button_code(),
                state: btn.state(),
                time_msec: btn.time_msec(),
            });
        }
        InputEvent::PointerAxis { event: axis } => {
            let horizontal = axis
                .amount(smithay::backend::input::Axis::Horizontal)
                .unwrap_or(0.0);
            let vertical = axis
                .amount(smithay::backend::input::Axis::Vertical)
                .unwrap_or(0.0);
            state.push_command(WmCommand::PointerAxis {
                source: axis.source(),
                horizontal,
                vertical,
                time_msec: axis.time_msec(),
            });
        }
        _ => {}
    }
}
