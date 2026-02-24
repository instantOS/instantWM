//! Interactive mouse-resize operations.
//!
//! Three distinct resize modes are provided:
//!
//! | Function                  | Description                                                  |
//! |---------------------------|--------------------------------------------------------------|
//! | [`resize_mouse`]          | Drag the bottom-right corner to resize                      |
//! | [`resize_aspect_mouse`]   | Same, but clamps to the window's declared aspect-ratio hints |
//! | [`force_resize_mouse`]    | Alias for `resize_mouse` (bypasses fullscreen guard)        |
//!
//! All three share the same grab/event-loop/ungrab skeleton; they differ only
//! in how they compute the new width and height from the pointer position.
//!
//! # Resize-direction constants
//!
//! The `RESIZE_DIR_*` constants in [`super::constants`] identify which corner
//! or edge is being dragged.  Currently only `RESIZE_DIR_BOTTOM_RIGHT` is used
//! by the interactive loops; the others are reserved for future directional
//! resize support.

use crate::client::resize;
use crate::floating::toggle_floating;
use crate::globals::get_globals;
use crate::monitor::is_current_layout_tiling;
use crate::types::*;
use crate::util::get_sel_win;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use super::constants::{REFRESH_RATE_HI, REFRESH_RATE_LO};
use super::grab::{grab_pointer, ungrab};
use super::monitor::handle_client_monitor_switch;

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Return the selected window for the current monitor, unless it is
/// non-fake-fullscreen (in which case resize is a no-op).
///
/// Returns `None` to signal "do nothing".
fn selected_resizable_window() -> Option<Window> {
    let win = get_sel_win()?;

    let globals = get_globals();
    let c = globals.clients.get(&win)?;
    if c.is_fullscreen && !c.isfakefullscreen {
        return None;
    }

    Some(win)
}

/// Decide the motion-event throttle based on `globals.doubledraw`.
fn refresh_rate() -> u32 {
    if get_globals().doubledraw {
        REFRESH_RATE_HI
    } else {
        REFRESH_RATE_LO
    }
}

// ── resize_mouse ─────────────────────────────────────────────────────────────

/// Interactive bottom-right-corner resize.
///
/// Grabs the pointer, then for every `MotionNotify` event computes a new
/// `(w, h)` from the distance between the pointer and the window's top-left
/// corner.  If the window is tiled and the delta exceeds the snap threshold,
/// it is promoted to floating first.
///
/// The loop ends on `ButtonRelease`.  After the grab is released,
/// [`handle_client_monitor_switch`] checks whether the window crossed a monitor
/// boundary during the resize.
pub fn resize_mouse(_arg: &Arg) {
    let Some(win) = selected_resizable_window() else {
        return;
    };

    let Some(conn) = grab_pointer(1) else { return };

    let (orig_left, orig_top) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.geo.x, c.geo.y),
            None => {
                ungrab(conn);
                return;
            }
        }
    };

    let rate = refresh_rate();
    let mut last_time: u32 = 0;

    loop {
        let Ok(event) = conn.wait_for_event() else {
            break;
        };

        match &event {
            x11rb::protocol::Event::ButtonRelease(_) => break,

            x11rb::protocol::Event::MotionNotify(m) => {
                if m.time - last_time <= 1000 / rate {
                    continue;
                }
                last_time = m.time;

                let nw = (m.event_x as i32 - orig_left + 1).max(1);
                let nh = (m.event_y as i32 - orig_top + 1).max(1);

                let globals = get_globals();
                let snap = globals.snap;

                if let Some(client) = globals.clients.get(&win) {
                    let has_tiling = globals
                        .monitors
                        .get(globals.selmon)
                        .map(|m| is_current_layout_tiling(m, &globals.tags))
                        .unwrap_or(true);

                    if !client.isfloating
                        && has_tiling
                        && ((nw - client.geo.w).abs() > snap || (nh - client.geo.h).abs() > snap)
                    {
                        toggle_floating(&Arg::default());
                    } else if !has_tiling || client.isfloating {
                        resize(
                            win,
                            &Rect {
                                x: client.geo.x,
                                y: client.geo.y,
                                w: nw,
                                h: nh,
                            },
                            true,
                        );
                    }
                }
            }

            _ => {}
        }
    }

    ungrab(conn);
    handle_client_monitor_switch(win);
}

/// Alias for [`resize_mouse`].
///
/// Exists to match the C API where `force_resize_mouse` was a separate symbol
/// that bypassed an additional fullscreen guard.  The Rust version already
/// handles this cleanly in [`selected_resizable_window`].
#[inline]
pub fn force_resize_mouse(arg: &Arg) {
    resize_mouse(arg);
}

// ── resize_aspect_mouse ───────────────────────────────────────────────────────

/// Interactive resize that respects the window's declared aspect-ratio hints.
///
/// Reads `client.mina`, `client.maxa`, and `client.size_hints` to clamp the
/// new dimensions so the window's aspect ratio stays within the range it
/// advertised via `WM_NORMAL_HINTS`.
///
/// Unlike [`resize_mouse`] this function does **not** toggle floating; it is
/// intended for use on windows that are already floating (e.g. video players
/// with a fixed aspect ratio).
pub fn resize_aspect_mouse(_arg: &Arg) {
    let globals = get_globals();
    let win = get_sel_win().filter(|&w| {
        !globals
            .clients
            .get(&w)
            .map(|c| c.is_fullscreen)
            .unwrap_or(false)
    });
    let Some(win) = win else {
        return;
    };

    let Some(conn) = grab_pointer(1) else { return };

    let (orig_left, orig_top) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.geo.x, c.geo.y),
            None => {
                ungrab(conn);
                return;
            }
        }
    };

    let rate = refresh_rate();
    let mut last_time: u32 = 0;

    loop {
        let Ok(event) = conn.wait_for_event() else {
            break;
        };

        match &event {
            x11rb::protocol::Event::ButtonRelease(_) => break,

            x11rb::protocol::Event::MotionNotify(m) => {
                if m.time - last_time <= 1000 / rate {
                    continue;
                }
                last_time = m.time;

                let raw_nw = (m.event_x as i32 - orig_left + 1).max(1);
                let raw_nh = (m.event_y as i32 - orig_top + 1).max(1);

                let globals = get_globals();
                if let Some(client) = globals.clients.get(&win) {
                    let sh = &client.size_hints;
                    let (mina, maxa) = (client.mina, client.maxa);

                    let mut nw = raw_nw;
                    let mut nh = raw_nh;

                    // Clamp to declared min/max dimensions.
                    if sh.minw > 0 {
                        nw = nw.max(sh.minw);
                    }
                    if sh.minh > 0 {
                        nh = nh.max(sh.minh);
                    }
                    if sh.maxw > 0 {
                        nw = nw.min(sh.maxw);
                    }
                    if sh.maxh > 0 {
                        nh = nh.min(sh.maxh);
                    }

                    // Clamp to declared aspect-ratio range.
                    if mina > 0.0 && maxa > 0.0 {
                        if maxa < nw as f32 / nh as f32 {
                            nw = (nh as f32 * maxa) as i32;
                        } else if mina < nh as f32 / nw as f32 {
                            nh = (nw as f32 * mina) as i32;
                        }
                    }

                    resize(
                        win,
                        &Rect {
                            x: client.geo.x,
                            y: client.geo.y,
                            w: nw,
                            h: nh,
                        },
                        true,
                    );
                }
            }

            _ => {}
        }
    }

    ungrab(conn);
    handle_client_monitor_switch(win);
}

// `hover_resize_mouse` and `is_in_resize_border` live in `super::hover`.
