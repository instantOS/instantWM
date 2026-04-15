//! Tag bar drag operations.
//!
//! This module handles dragging across the tag bar to switch views or move
//! windows between tags.

use crate::backend::x11::grab::mouse_drag_loop;
use crate::bar::bar_position_to_gesture;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::mouse::cursor::set_cursor_style;
use crate::types::*;
use x11rb::protocol::xproto::ModMask;

/// Begin a tag-bar drag. Returns `true` if a drag was started (current tag
/// clicked with a selected window), `false` if the click was fully handled
/// (view switch or no selection).
///
/// On Wayland the caller should return after this — the calloop will drive
/// [`drag_tag_motion`] and [`drag_tag_finish`].  On X11 the caller enters a
/// grab loop that calls those two functions synchronously.
pub fn drag_tag_begin(ctx: &mut WmCtx, bar_pos: BarPosition, btn: MouseButton) -> bool {
    let selmon_id = ctx.core().globals().selected_monitor_id();
    let mon_mx = ctx.core().globals().selected_monitor().work_rect.x;

    let initial_tag = match bar_pos {
        BarPosition::Tag(idx) => 1u32 << (idx as u32),
        _ => {
            let ptr_x = ctx.pointer_location().map(|(x, _)| x).unwrap_or(0);
            let core = ctx.core();
            core.globals()
                .monitors
                .get(selmon_id)
                .and_then(|mon| {
                    let local_x = ptr_x - mon.work_rect.x;
                    match mon.bar_position_at_x(core, local_x) {
                        BarPosition::Tag(idx) => Some(1u32 << (idx as u32)),
                        _ => None,
                    }
                })
                .unwrap_or(0)
        }
    };

    let current_tagset = ctx.core().globals().selected_monitor().selected_tags();
    let is_current_tag =
        (TagMask::from_bits(initial_tag) & ctx.core().globals().tags.mask()) == current_tagset;
    let has_sel = ctx.selected_client().is_some();

    // Click on a *different* tag → switch view, no drag.
    if !is_current_tag && initial_tag != 0 {
        crate::tags::view::view_tags(ctx, TagMask::from_bits(initial_tag));
        return false;
    }
    // No selected window → nothing to drag.
    if !has_sel {
        return false;
    }

    // Initialise the drag state machine.
    ctx.core_mut().globals_mut().drag.tag = crate::globals::TagDragState {
        active: true,
        initial_tag,
        monitor_id: selmon_id,
        mon_mx,
        last_tag: -1,
        cursor_on_bar: true,
        last_motion: None,
        button: btn,
    };
    set_cursor_style(ctx, AltCursor::Move);
    ctx.core_mut().globals_mut().drag.bar_active = true;
    ctx.request_bar_update(Some(selmon_id));
    true
}

/// Process a single motion event during an active tag drag.
///
/// Updates gesture highlighting and detects when the cursor leaves the bar.
/// Returns `false` if the cursor left the bar (caller should finish the drag).
pub fn drag_tag_motion(ctx: &mut WmCtx, root_x: i32, root_y: i32) -> bool {
    if !ctx.core().globals().drag.tag.active {
        return false;
    }

    let selmon_id = ctx.core_mut().globals_mut().drag.tag.monitor_id;
    let mon_mx = ctx.core_mut().globals_mut().drag.tag.mon_mx;

    let bar_bottom = {
        let mon = ctx.core_mut().globals_mut().selected_monitor();
        mon.bar_y + mon.bar_height + 1
    };

    if root_y > bar_bottom {
        ctx.core_mut().globals_mut().drag.tag.cursor_on_bar = false;
        return false;
    }

    // Store last motion for release handling.  Modifier state is not available
    // from root coords alone; the caller sets it via drag_tag_finish.
    ctx.core_mut().globals_mut().drag.tag.last_motion = Some((root_x, root_y, 0));

    let local_x = root_x - mon_mx;
    let new_gesture = {
        let core = ctx.core();
        core.globals()
            .monitors
            .get(selmon_id)
            .map(|mon| bar_position_to_gesture(mon.bar_position_at_x(core, local_x)))
            .unwrap_or(Gesture::None)
    };
    let gesture_key = match new_gesture {
        Gesture::Tag(idx) => idx as i32,
        _ => -1,
    };

    if ctx.core_mut().globals_mut().drag.tag.last_tag != gesture_key {
        ctx.core_mut().globals_mut().drag.tag.last_tag = gesture_key;
        if let Some(mon) = ctx.core_mut().globals_mut().monitors.get_mut(selmon_id) {
            mon.gesture = new_gesture;
        }
        ctx.request_bar_update(Some(selmon_id));
    }
    true
}

/// Finish a tag drag: apply the action based on the final position and
/// modifier keys held at release time.
///
/// `modifier_state` is the X11-style modifier bitmask at release time
/// (Shift, Control, …).
pub fn drag_tag_finish(ctx: &mut WmCtx, modifier_state: u32) {
    if !ctx.core().globals().drag.tag.active {
        return;
    }

    let selmon_id = ctx.core_mut().globals_mut().drag.tag.monitor_id;
    let cursor_on_bar = ctx.core_mut().globals_mut().drag.tag.cursor_on_bar;
    let last_motion = ctx.core_mut().globals_mut().drag.tag.last_motion;

    // Clear state first so re-entrant calls are safe.
    ctx.core_mut().globals_mut().drag.tag.active = false;

    if cursor_on_bar && let Some((x, _, _)) = last_motion {
        let position = {
            let core = ctx.core();
            let mon = core.globals().selected_monitor();
            let local_x = x - mon.work_rect.x;
            mon.bar_position_at_x(core, local_x)
        };

        if let BarPosition::Tag(tag_idx) = position {
            let tag_mask = TagMask::single(tag_idx + 1).unwrap_or(TagMask::EMPTY);
            if (modifier_state & ModMask::SHIFT.bits() as u32) != 0 {
                if let Some(win) = ctx
                    .core_mut()
                    .globals_mut()
                    .monitor(selmon_id)
                    .and_then(|m| m.sel)
                {
                    crate::tags::client_tags::set_client_tag(ctx, win, tag_mask);
                }
            } else if (modifier_state & ModMask::CONTROL.bits() as u32) != 0 {
                crate::tags::client_tags::tag_all_ctx(ctx, tag_mask);
            } else if let Some(win) = ctx
                .core_mut()
                .globals_mut()
                .monitor(selmon_id)
                .and_then(|m| m.sel)
            {
                crate::tags::client_tags::follow_tag_ctx(ctx, win, tag_mask);
            }
        }
    }

    ctx.core_mut().globals_mut().drag.bar_active = false;
    if let Some(mon) = ctx.core_mut().globals_mut().monitor_mut(selmon_id) {
        mon.gesture = Gesture::None;
    }
    set_cursor_style(ctx, AltCursor::Default);
    ctx.request_bar_update(Some(selmon_id));
}

/// Drag across the tag bar to switch the view or move/follow a window to a tag.
///
/// * Plain click on a different tag   → [`view`]
/// * Plain click on the current tag   → drag; release with `Shift` → [`set_client_tag_ctx`],
///   `Control` → [`tag_all_ctx`], no modifier → [`follow_tag_ctx`]
///
/// Exits without action if the pointer leaves the bar during the drag.
///
/// On X11, runs a synchronous grab loop.  On Wayland, starts the drag and
/// returns immediately — the calloop drives [`drag_tag_motion`] and
/// [`drag_tag_finish`].
pub fn drag_tag(ctx: &mut WmCtxX11, bar_pos: BarPosition, btn: MouseButton, _click_root_x: i32) {
    if !{
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        drag_tag_begin(&mut wm_ctx, bar_pos, btn)
    } {
        return;
    }

    // ── X11 synchronous grab loop ─────────────────────────────────────────
    mouse_drag_loop(ctx, btn, AltCursor::Move, false, |ctx, event| {
        if let x11rb::protocol::Event::MotionNotify(m) = event {
            // Update stored modifier state from latest motion.
            let root_x = m.event_x as i32;
            let root_y = m.event_y as i32;
            let mod_state = u16::from(m.state) as u32;

            // Store motion with modifier state for release handling.
            ctx.core.globals_mut().drag.tag.last_motion = Some((root_x, root_y, mod_state));

            let mut wm_ctx = WmCtx::X11(ctx.reborrow());
            return drag_tag_motion(&mut wm_ctx, root_x, root_y);
        }
        true
    });

    let modifier_state = {
        ctx.core
            .globals()
            .drag
            .tag
            .last_motion
            .map(|(_, _, m)| m)
            .unwrap_or(0)
    };

    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    drag_tag_finish(&mut wm_ctx, modifier_state);
}
