//! Floating state transitions and geometry persistence.
//!
//! This module owns everything that **changes** whether a window is floating
//! and everything that **saves / restores** the geometry associated with that
//! state.
//!
//! # Concepts
//!
//! - **float_geo** (`Client::float_geo`) — the last known floating rect.
//!   Saved when entering tiling or snap; restored when returning to free float.
//! - **border_width / old_border_width** — maximized-snap zeroes the border;
//!   these fields let us round-trip the original value.
//! - **`apply_float_change`** — the single internal function that actually
//!   flips `isfloating`.  All public toggle/set helpers funnel through it.
//!
//! # Public surface
//!
//! | Function                | Purpose                                              |
//! |-------------------------|------------------------------------------------------|
//! | `save_floating_win`     | snapshot current geometry into `float_geo`           |
//! | `restore_floating_win`  | resize window to its saved `float_geo`               |
//! | `save_bw_win`           | snapshot border width into `old_border_width`        |
//! | `restore_border_width_win` | write `old_border_width` back to `border_width`   |
//! | `toggle_floating`       | flip the selected window's floating state (animated) |
//! | `change_floating_win`   | flip any window's floating state (no animation)      |
//! | `set_floating`          | make a window floating if it isn't already           |
//! | `set_tiled`             | make a window tiled if it isn't already              |
//! | `temp_fullscreen`       | toggle quick fullscreen outside the EWMH protocol    |

use crate::animation::animate_client_rect;
use crate::client::resize;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::monitor::arrange;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

// ── Geometry persistence ──────────────────────────────────────────────────────

/// Snapshot the window's current geometry into `Client::float_geo`.
///
/// Call this before entering tiling or snap so that the position can be
/// recovered when the window becomes freely floating again.
pub fn save_floating_win(win: Window) {
    let globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        client.float_geo = client.geo;
    }
}

/// Resize the window to its previously saved `float_geo`.
///
/// Has no effect if the window has no saved geometry (e.g. it was never
/// floating before) because `float_geo` will simply equal a zero rect.
pub fn restore_floating_win(win: Window) {
    let float_geo = {
        let globals = get_globals();
        globals.clients.get(&win).map(|c| c.float_geo)
    };
    if let Some(rect) = float_geo {
        resize(win, &rect, false);
    }
}

// ── Border width persistence ──────────────────────────────────────────────────

/// Snapshot the current border width into `Client::old_border_width`.
///
/// Called before maximized-snap zeroes the border so it can be restored later.
/// Does nothing if the current border width is already zero.
pub fn save_bw_win(win: Window) {
    let globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        if client.border_width != 0 {
            client.old_border_width = client.border_width;
        }
    }
}

/// Write `old_border_width` back to `border_width`.
///
/// Called when leaving maximized-snap to undo the border zeroing.
/// Does nothing if no border width was previously saved.
pub fn restore_border_width_win(win: Window) {
    let globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        if client.old_border_width != 0 {
            client.border_width = client.old_border_width;
        }
    }
}

// ── Core state transition ─────────────────────────────────────────────────────

/// Flip a window's `isfloating` flag and handle all side-effects.
///
/// This is the single function that actually changes floating state.  All
/// public helpers (`toggle_floating`, `set_floating`, …) delegate here.
///
/// # Parameters
///
/// - `floating`        — desired new state (`true` = floating, `false` = tiled)
/// - `animate`         — play an animation when restoring the floating rect
/// - `update_borders`  — repaint border and handle border-width bookkeeping
///
/// Does **not** call `arrange()`; the caller is responsible for that.
pub fn apply_float_change(win: Window, floating: bool, animate: bool, update_borders: bool) {
    if floating {
        {
            let globals = get_globals_mut();
            if let Some(client) = globals.clients.get_mut(&win) {
                client.isfloating = true;
            }
        }

        if update_borders {
            restore_border_width_win(win);

            let x11 = get_x11();
            if let Some(ref conn) = x11.conn {
                let globals = get_globals();
                if let Some(ref scheme) = globals.borderscheme {
                    let pixel = scheme.float_focus.bg.color.pixel;
                    let _ = change_window_attributes(
                        conn,
                        win,
                        &ChangeWindowAttributesAux::new().border_pixel(Some(pixel as u32)),
                    );
                    let _ = conn.flush();
                }
            }
        }

        let saved_geo = {
            let globals = get_globals();
            globals.clients.get(&win).map(|c| c.float_geo)
        };

        let Some(saved_geo) = saved_geo else { return };

        if animate {
            animate_client_rect(win, &saved_geo, 7, 0);
        } else {
            resize(win, &saved_geo, false);
        }
    } else {
        // Switching to tiled: persist the current floating rect so we can
        // restore it if the window goes back to floating later.
        let client_count = get_globals().clients.len();
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.isfloating = false;
            client.float_geo = client.geo;

            if update_borders {
                // Single-window layouts don't need a visible border.
                if client_count <= 1 && client.snapstatus == SnapPosition::None {
                    client.old_border_width = client.border_width;
                    client.border_width = 0;
                }
                // Border repaint is handled by the caller (arrange → drawbar).
            }
        }
    }
}

// ── Public toggle / set helpers ───────────────────────────────────────────────

/// Flip the **selected** window's floating state with animation and border
/// updates.
///
/// This is the user-facing command (bound to a key or mouse button).
/// It does nothing if:
/// - no window is selected
/// - the selected window is the overlay
/// - the window is in true fullscreen (not fake-fullscreen)
pub fn toggle_floating(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        let mon = match globals.monitors.get(globals.selmon) {
            Some(m) => m,
            None => return,
        };
        match mon.sel {
            Some(sel) if Some(sel) != mon.overlay => {
                if let Some(c) = globals.clients.get(&sel) {
                    if c.is_fullscreen && !c.isfakefullscreen {
                        return;
                    }
                }
                Some(sel)
            }
            _ => None,
        }
    };

    let Some(win) = sel_win else { return };

    let (is_floating, is_fixed) = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| (c.isfloating, c.isfixed))
            .unwrap_or((false, false))
    };

    // A fixed window is always floating even if `isfloating` is false.
    let new_state = !is_floating || is_fixed;
    apply_float_change(win, new_state, true, true);
    arrange(Some(get_globals().selmon));
}

/// Flip **any** window's floating state without animation or border updates.
///
/// Unlike [`toggle_floating`] this:
/// - accepts an explicit window instead of using `selmon->sel`
/// - skips the animation (uses `resize` directly)
/// - skips border colour / width updates
///
/// Used primarily for overlay windows where visual effects aren't desired.
pub fn change_floating_win(win: Window) {
    let (is_fullscreen, is_fake_fullscreen, is_floating, is_fixed) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.is_fullscreen, c.isfakefullscreen, c.isfloating, c.isfixed),
            None => return,
        }
    };

    if is_fullscreen && !is_fake_fullscreen {
        return;
    }

    let new_state = !is_floating || is_fixed;
    apply_float_change(win, new_state, false, false);
    arrange(Some(get_globals().selmon));
}

/// Make a window floating if it is not already.
///
/// Does nothing when:
/// - the window is in true fullscreen
/// - the window is already floating
///
/// `should_arrange` triggers an `arrange()` pass after the state change, which
/// is usually what you want unless you are batching multiple changes.
pub fn set_floating(win: Window, should_arrange: bool) {
    let (is_fullscreen, is_fake_fullscreen, is_floating) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.is_fullscreen, c.isfakefullscreen, c.isfloating),
            None => return,
        }
    };

    if is_fullscreen && !is_fake_fullscreen {
        return;
    }
    if is_floating {
        return; // already floating, nothing to do
    }

    apply_float_change(win, true, false, false);

    if should_arrange {
        arrange(Some(get_globals().selmon));
    }
}

/// Make a window tiled if it is not already.
///
/// Does nothing when:
/// - the window is in true fullscreen
/// - the window is already tiled (and not fixed)
///
/// `should_arrange` triggers an `arrange()` pass after the state change.
pub fn set_tiled(win: Window, should_arrange: bool) {
    let (is_fullscreen, is_fake_fullscreen, is_floating, is_fixed) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.is_fullscreen, c.isfakefullscreen, c.isfloating, c.isfixed),
            None => return,
        }
    };

    if is_fullscreen && !is_fake_fullscreen {
        return;
    }
    if !is_floating && !is_fixed {
        return; // already tiled, nothing to do
    }

    apply_float_change(win, false, false, false);

    if should_arrange {
        arrange(Some(get_globals().selmon));
    }
}

// ── Temporary fullscreen ──────────────────────────────────────────────────────

/// Toggle quick fullscreen for the selected window.
///
/// "Temporary" fullscreen differs from EWMH fullscreen
/// (`_NET_WM_STATE_FULLSCREEN`) in several ways:
///
/// - State is tracked in `Monitor::fullscreen`, not in the client's
///   `is_fullscreen` flag, so it does not interfere with EWMH consumers.
/// - Entering fullscreen saves the current floating geometry; leaving restores
///   it (for floating windows) or lets the layout engine re-tile the window.
/// - Animations are temporarily disabled during the transition so the switch
///   feels instant.
/// - The window is raised above all siblings when entering fullscreen.
///
/// Typical use: a keybinding for quick "distraction-free" mode that the user
/// can toggle without the window losing its place in the layout.
pub fn temp_fullscreen(_arg: &Arg) {
    let (fullscreen_win, sel_win, animated) = {
        let globals = get_globals();
        let mon = match globals.monitors.get(globals.selmon) {
            Some(m) => m,
            None => return,
        };
        (mon.fullscreen, mon.sel, globals.animated)
    };

    if let Some(win) = fullscreen_win {
        // ── Leaving fullscreen ────────────────────────────────────────────
        let is_floating = {
            let globals = get_globals();
            globals
                .clients
                .get(&win)
                .map(|c| c.isfloating)
                .unwrap_or(false)
        };

        if is_floating || !super::helpers::has_tiling_layout() {
            restore_floating_win(win);
            super::helpers::apply_size(win);
        }

        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            mon.fullscreen = None;
        }
    } else {
        // ── Entering fullscreen ───────────────────────────────────────────
        let Some(win) = sel_win else { return };

        {
            let globals = get_globals_mut();
            if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
                mon.fullscreen = Some(win);
            }
        }

        // Save geometry so we can restore it when leaving.
        if super::helpers::check_floating(win) {
            save_floating_win(win);
        }
    }

    // Disable animations for the instant layout switch, then restore.
    if animated {
        get_globals_mut().animated = false;
        arrange(Some(get_globals().selmon));
        get_globals_mut().animated = true;
    } else {
        arrange(Some(get_globals().selmon));
    }

    // Raise the fullscreen window above everything else.
    if let Some(win) = get_globals()
        .monitors
        .get(get_globals().selmon)
        .and_then(|m| m.fullscreen)
    {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = configure_window(
                conn,
                win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
            let _ = conn.flush();
        }
    }
}
