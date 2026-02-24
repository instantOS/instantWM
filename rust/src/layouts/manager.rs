//! Layout manager — the stateful half of the layout system.
//!
//! This module owns every operation that *changes* layout state or triggers a
//! full re-arrange pass.  Pure geometry algorithms live in [`super::algo`];
//! stateless queries live in [`super::query`].
//!
//! ## Responsibilities
//!
//! | Function                  | What it does                                               |
//! |---------------------------|------------------------------------------------------------|
//! | [`arrange`]               | Top-level arrange: show/hide clients, then arrange all monitors (or one) |
//! | [`arrange_monitor`]       | Arrange a single monitor: update border widths, call layout, place overlay |
//! | [`restack`]               | Correct the X11 window stacking order for a monitor        |
//! | [`set_layout`]            | Switch the active layout (handles tagprefix / multimon)    |
//! | [`set_layout_by_index`]   | Set layout by raw index (ergonomic helper)                 |
//! | [`cycle_layout`]          | Legacy `&Arg` wrapper for key bindings                     |
//! | [`cycle_layout_direction`]| Typed forward/backward cycle, skipping overview            |
//! | [`inc_nmaster`]           | Legacy `&Arg` wrapper for key bindings                     |
//! | [`inc_nmaster_by`]        | Adjust master-client count by a signed delta               |
//! | [`set_mfact`]             | Adjust the master-area fraction                            |
//! | [`command_layout`]        | IPC command handler — set layout by 1-based `arg.ui`       |

use crate::bar::draw_bar;
use crate::client::{next_tiled, resize, restore_border_width, save_border_width};
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::layouts::algo::save_floating;
use crate::layouts::query::{
    client_count, client_count_mon, get_current_layout, get_current_layout_idx, is_monocle_layout,
    is_overview_layout, is_tiling_layout,
};
use crate::types::{Arg, Monitor, MonitorId, Rect};
use crate::util::max;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

// ── thin local delegating helpers ────────────────────────────────────────────

/// Delegate to [`crate::mouse::reset_cursor`].
fn reset_cursor() {
    crate::mouse::reset_cursor();
}

/// Delegate to [`crate::client::show_hide`].
fn show_hide(win: Option<Window>) {
    crate::client::show_hide(win);
}

// ── arrange ───────────────────────────────────────────────────────────────────

/// Top-level arrange entry point.
///
/// When `mon_id` is `Some(id)`, only that monitor is arranged (show/hide +
/// arrange + restack).  When it is `None`, every monitor is arranged (useful
/// after a tag or layout change that might affect all monitors).
///
/// Always resets the cursor first so stale resize/move cursors are cleared.
pub fn arrange(mon_id: Option<MonitorId>) {
    reset_cursor();

    if let Some(id) = mon_id {
        // ── single monitor fast-path ──────────────────────────────────────
        {
            let g = get_globals_mut();
            if let Some(m) = g.monitors.get_mut(id) {
                let stack = m.stack;
                show_hide(stack);
            }
        }
        {
            let g = get_globals_mut();
            if let Some(m) = g.monitors.get_mut(id) {
                arrange_monitor(m);
                restack(m);
            }
        }
    } else {
        // ── all monitors ──────────────────────────────────────────────────
        // Collect stacks first to avoid holding a borrow during show_hide.
        let stacks: Vec<Option<Window>> = {
            let g = get_globals();
            g.monitors.iter().map(|m| m.stack).collect()
        };

        for stack in stacks {
            show_hide(stack);
        }

        let g = get_globals_mut();
        for m in g.monitors.iter_mut() {
            arrange_monitor(m);
        }
    }
}

// ── arrange_monitor ───────────────────────────────────────────────────────────

/// Arrange a single monitor.
///
/// Steps performed:
///
/// 1. **Update `clientcount`** — count tiled, visible clients so layout
///    algorithms and bar rendering have an up-to-date number.
/// 2. **Border-width adjustment** — single-client tiling and monocle layouts
///    strip the border; all other cases restore the saved border width.
/// 3. **Run the layout algorithm** — delegates to the active [`Layout`]
///    trait object, which calls the appropriate `algo::*` function.
/// 4. **Place the overlay window** — if the monitor has an overlay client it
///    is stretched to fill the full monitor rect (minus bar, if shown).
///
/// [`Layout`]: crate::types::Layout
pub fn arrange_monitor(m: &mut Monitor) {
    m.clientcount = client_count_mon(m) as u32;
    apply_border_widths(m);
    run_layout(m);
    place_overlay(m);
}

/// Adjust border widths for all tiled clients on `m`.
///
/// Borders are stripped (set to 0) when a client would fill the entire tiling
/// area anyway — specifically when it is the only tiled client in a tiling
/// layout, or when the active layout is monocle.  In all other cases the
/// previously-saved border width is restored.
///
/// The layout flags (`is_tiling`, `is_monocle`) are read once before the loop
/// because every client on this monitor shares the same layout.
fn apply_border_widths(m: &Monitor) {
    // Read layout properties once — all tiled clients on this monitor share them.
    let is_tiling = is_tiling_layout(m);
    let is_monocle = is_monocle_layout(m);
    let clientcount = m.clientcount;

    let mut c_win = next_tiled(m.clients);
    while let Some(win) = c_win {
        let (is_floating, is_fullscreen) = {
            let g = get_globals();
            match g.clients.get(&win) {
                None => break,
                Some(c) => (c.isfloating, c.is_fullscreen),
            }
        };

        // Strip border when a single tiled client fills the whole area, or
        // in monocle mode (where all clients fill the area anyway).
        let strip_border =
            !is_floating && !is_fullscreen && ((clientcount == 1 && is_tiling) || is_monocle);

        if strip_border {
            save_border_width(win);
            if let Some(c) = get_globals_mut().clients.get_mut(&win) {
                c.border_width = 0;
            }
        } else {
            restore_border_width(win);
        }

        c_win = get_globals()
            .clients
            .get(&win)
            .and_then(|c| next_tiled(c.next));
    }
}

/// Run the active layout algorithm for `m`.
fn run_layout(m: &mut Monitor) {
    get_current_layout(m).arrange(m);
}

/// Place the overlay window (if any) so it fills the monitor work area.
///
/// The overlay always occupies the full monitor rect, inset only by the bar
/// height when the bar is visible.  `work_rect` already encodes this, so we
/// derive the overlay geometry directly from it rather than re-computing the
/// bar offset manually.
fn place_overlay(m: &mut Monitor) {
    let overlay_win = match m.overlay {
        Some(w) => w,
        None => return,
    };

    let g = get_globals_mut();
    if let Some(c) = g.clients.get_mut(&overlay_win) {
        if c.isfloating {
            save_floating(overlay_win);
        }
    }

    // work_rect already has y nudged past the bar (top-bar) and h reduced by
    // bh, so no manual bar-offset arithmetic is needed here.
    let bw = g.clients.get(&overlay_win).map_or(0, |c| c.border_width);
    let geo = Rect {
        x: m.work_rect.x,
        y: m.work_rect.y,
        w: m.work_rect.w - 2 * bw,
        h: m.work_rect.h - 2 * bw,
    };

    resize(overlay_win, &geo, false);
}

// ── restack ───────────────────────────────────────────────────────────────────

/// Correct the X11 window stacking order for monitor `m`.
///
/// In tiling layouts, tiled windows are pushed below the bar window so that
/// floating windows (which are raised above the stack) stay on top.  In
/// floating layouts, only the selected window is raised.
///
/// Does nothing when the monitor is in overview mode (the overview layout
/// manages its own Z-order).
pub fn restack(m: &mut Monitor) {
    if is_overview_layout(m) {
        return;
    }

    draw_bar(m);

    let sel_win = match m.sel {
        Some(w) => w,
        None => return,
    };

    let is_tiling = get_current_layout(m).is_tiling();

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let is_floating = get_globals()
            .clients
            .get(&sel_win)
            .map(|c| c.isfloating)
            .unwrap_or(false);

        // Raise floating windows (or any window in a non-tiling layout) so
        // they appear above tiled windows.
        if is_floating || !is_tiling {
            let _ = configure_window(
                conn,
                sel_win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
        }

        // In tiling layouts, push each non-floating, visible client below the
        // bar so floating windows on top remain unobscured.
        if is_tiling {
            let mut wc = ConfigureWindowAux::new()
                .stack_mode(StackMode::BELOW)
                .sibling(m.barwin);

            let mut s_win = m.stack;
            while let Some(win) = s_win {
                let g = get_globals();
                match g.clients.get(&win) {
                    None => break,
                    Some(c) => {
                        let is_win_floating = c.isfloating;
                        let visible = c.is_visible();
                        let snext = c.snext;

                        if !is_win_floating && visible {
                            let _ = configure_window(conn, win, &wc);
                            // Each subsequent window goes above the one we just placed,
                            // building the correct tiled stack from the bottom up.
                            wc = ConfigureWindowAux::new()
                                .stack_mode(StackMode::ABOVE)
                                .sibling(win);
                        }

                        s_win = snext;
                    }
                }
            }
        }

        let _ = conn.flush();
    }
}

// ── set_layout ────────────────────────────────────────────────────────────────

/// Switch the active layout on the selected monitor (or all monitors when
/// `tagprefix` is set).
///
/// The `arg.v` field encodes the desired layout index (`Some(idx)`) or a
/// toggle request (`None`, which flips `sellt` to the previously used layout).
///
/// ## tagprefix mode
///
/// When `globals.tags.prefix` is `true` the layout change is applied to
/// *every* tag on *every* monitor, then `prefix` is cleared.  This is used
/// by the "apply to all" prefix key binding.
pub fn set_layout(arg: &Arg) {
    let tagprefix = get_globals().tags.prefix;

    if tagprefix {
        // ── broadcast to all tags ─────────────────────────────────────────
        {
            let g = get_globals_mut();
            for tag in g.tags.tags.iter_mut() {
                if arg.v.is_none() {
                    tag.sellt ^= 1;
                }
                if let Some(idx) = arg.v {
                    tag.ltidxs[tag.sellt as usize] = Some(idx);
                }
            }
            g.tags.prefix = false;
        }
        // Re-enter to handle the current monitor arrange.
        set_layout(arg);
        return;
    }

    // ── apply to the current tag on the selected monitor ──────────────────
    {
        let g = get_globals_mut();
        if let Some(m) = g.monitors.get_mut(g.selmon) {
            let current_tag = m.current_tag;
            if current_tag > 0 && current_tag <= g.tags.tags.len() {
                let tag = &mut g.tags.tags[current_tag - 1];
                let current_idx = tag.ltidxs[tag.sellt as usize];

                if arg.v.is_none() || arg.v != current_idx {
                    tag.sellt ^= 1;
                }
                if let Some(idx) = arg.v {
                    tag.ltidxs[tag.sellt as usize] = Some(idx);
                }
            }
        }
    }

    // ── trigger arrange or at minimum redraw the bar ──────────────────────
    let (selmon, sel) = {
        let g = get_globals();
        let sel = g.monitors.get(g.selmon).and_then(|m| m.sel);
        (g.selmon, sel)
    };

    if sel.is_some() {
        arrange(Some(selmon));
    } else {
        let g = get_globals_mut();
        if let Some(m) = g.monitors.get_mut(selmon) {
            draw_bar(m);
        }
    }
}

// ── set_layout_by_index ───────────────────────────────────────────────────────

/// Set the layout directly by its index in `globals.layouts`.
///
/// Prefer this over calling [`set_layout`] with a raw [`Arg`] at call-sites
/// that already have a typed `usize` index (e.g. after cycling or parsing an
/// IPC command).
pub fn set_layout_by_index(layout_idx: usize) {
    set_layout(&Arg {
        v: Some(layout_idx),
        ..Default::default()
    });
}

// ── cycle_layout_direction ────────────────────────────────────────────────────

/// Cycle to the next (`forward = true`) or previous (`forward = false`) layout.
///
/// The overview layout is always skipped during cycling — it can only be
/// entered explicitly via a dedicated key binding or command.
pub fn cycle_layout_direction(forward: bool) {
    let (current_idx, layouts_len) = {
        let g = get_globals();
        let idx = g.monitors.get(g.selmon).and_then(get_current_layout_idx);
        (idx, g.layouts.len())
    };

    if layouts_len == 0 {
        return;
    }

    let current = current_idx.unwrap_or(0);

    // Compute naive next/prev index with wrap-around.
    let candidate = if forward {
        (current + 1) % layouts_len
    } else if current == 0 {
        layouts_len - 1
    } else {
        current - 1
    };

    // Skip over the overview layout — one extra step in the same direction.
    let skip = {
        let g = get_globals();
        g.layouts.get(candidate).is_some_and(|l| l.is_overview())
    };

    let final_idx = if skip {
        if forward {
            (candidate + 1) % layouts_len
        } else if candidate == 0 {
            layouts_len - 1
        } else {
            candidate - 1
        }
    } else {
        candidate
    };

    set_layout_by_index(final_idx);
}

// ── cycle_layout (legacy &Arg wrapper) ───────────────────────────────────────

/// Legacy key-binding wrapper.
///
/// `arg.i > 0` → forward, `arg.i <= 0` → backward.
/// New code should call [`cycle_layout_direction`] directly.
pub fn cycle_layout(arg: &Arg) {
    cycle_layout_direction(arg.i > 0);
}

// ── inc_nmaster_by ────────────────────────────────────────────────────────────

/// Adjust the number of master-area clients by `delta`.
///
/// - Positive delta increases `nmaster` (capped at the current client count).
/// - Negative delta decreases `nmaster` (floored at 0).
///
/// The new value is persisted on both the monitor and the active tag so it
/// survives tag switching.
pub fn inc_nmaster_by(delta: i32) {
    let ccount = client_count();

    {
        let g = get_globals_mut();
        if let Some(m) = g.monitors.get_mut(g.selmon) {
            // Guard against increasing beyond the number of visible clients.
            if delta > 0 && m.nmaster >= ccount {
                m.nmaster = ccount;
                return;
            }

            let new_nmaster = max(m.nmaster + delta, 0);
            m.nmaster = new_nmaster;

            let tag = m.current_tag;
            if tag > 0 && tag <= g.tags.tags.len() {
                g.tags.tags[tag - 1].nmaster = new_nmaster;
            }
        }
    }

    let selmon = get_globals().selmon;
    arrange(Some(selmon));
}

// ── inc_nmaster (legacy &Arg wrapper) ────────────────────────────────────────

/// Legacy key-binding wrapper.  New code should call [`inc_nmaster_by`] directly.
pub fn inc_nmaster(arg: &Arg) {
    inc_nmaster_by(arg.i);
}

// ── set_mfact ─────────────────────────────────────────────────────────────────

/// Set the master-area width/height fraction.
///
/// `arg.f` semantics (mirroring dwm):
///
/// - `0.0`         → no-op.
/// - `0.0 < f < 1.0` → *delta*: the value is added to the current mfact.
/// - `f >= 1.0`    → *absolute*: `f - 1.0` is used as the new mfact.
///
/// The result is clamped to `[0.05, 0.95]` so the master area never
/// disappears entirely.  Animation is briefly disabled during the resize to
/// avoid a visually jarring multi-frame transition when dragging the split.
pub fn set_mfact(arg: &Arg) {
    if arg.f == 0.0 {
        return;
    }

    // Only applicable to tiling layouts.
    let is_tiling = {
        let g = get_globals();
        g.monitors
            .get(g.selmon)
            .map(|m| get_current_layout(m).is_tiling())
            .unwrap_or(false)
    };

    if !is_tiling {
        return;
    }

    let current_mfact = {
        let g = get_globals();
        g.monitors.get(g.selmon).map(|m| m.mfact).unwrap_or(0.55)
    };

    // Resolve delta vs absolute.
    let new_mfact = if arg.f < 1.0 {
        arg.f + current_mfact
    } else {
        arg.f - 1.0
    };

    if !(0.05..=0.95).contains(&new_mfact) {
        return;
    }

    // Disable animation for the arrange triggered by this mfact change so
    // the split line moves without a distracting multi-frame animation.
    let animation_on = get_globals().animated && client_count() > 2;
    if animation_on {
        get_globals_mut().animated = false;
    }

    {
        let g = get_globals_mut();
        if let Some(m) = g.monitors.get_mut(g.selmon) {
            m.mfact = new_mfact;
            let tag = m.current_tag;
            if tag > 0 && tag <= g.tags.tags.len() {
                g.tags.tags[tag - 1].mfact = new_mfact;
            }
        }
    }

    let selmon = get_globals().selmon;
    arrange(Some(selmon));

    if animation_on {
        get_globals_mut().animated = true;
    }
}

// ── command_layout ────────────────────────────────────────────────────────────

/// IPC command handler: set the layout to `arg.ui` (1-based index clamped to
/// the available layout list).
///
/// `arg.ui == 0` is treated as index 0 (first layout).  Out-of-range values
/// are also clamped to 0.
pub fn command_layout(arg: &Arg) {
    let layouts_len = get_globals().layouts.len();
    let idx = if arg.ui > 0 && (arg.ui as usize) < layouts_len {
        arg.ui as usize
    } else {
        0
    };

    set_layout_by_index(idx);
}
