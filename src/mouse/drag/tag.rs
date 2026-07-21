//! Tag bar drag operations.
//!
//! This module handles dragging across the tag bar to switch views or move
//! windows between tags.

use crate::backend::BackendEvent;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::mouse::cursor::set_cursor_style;
use crate::types::*;

// X11 modifier mask constants — these are standard across all X11 implementations.
const SHIFT_MASK: u32 = 1;
const CTRL_MASK: u32 = 4;

/// Begin a tag-bar drag. Returns `true` if a drag was started (current tag
/// clicked with a selected window), `false` if the click was fully handled
/// (view switch or no selection).
///
/// On Wayland the caller should return after this — the calloop will drive
/// [`drag_tag_motion`] and [`drag_tag_finish`].  On X11 the caller enters a
/// grab loop that calls those two functions synchronously.
pub fn drag_tag_begin(ctx: &mut WmCtx, bar_pos: BarPosition, btn: MouseButton) -> bool {
    let selmon_id = ctx.core().model().selected_monitor_id();
    let mon_mx = ctx.core().model().expect_selected_monitor().work_rect().x;

    let initial_tag = match bar_pos {
        BarPosition::Tag(idx) => TagMask::from_index(idx).unwrap_or(TagMask::EMPTY),
        _ => {
            let ptr_x = ctx
                .pointer_backend()
                .pointer_location()
                .map(|p| p.x)
                .unwrap_or(0);
            let core = ctx.core();
            core.state()
                .model
                .monitors
                .get(selmon_id)
                .and_then(|mon| {
                    let local_x = ptr_x - mon.work_rect().x;
                    match mon.bar_position_at_x(core, local_x) {
                        BarPosition::Tag(idx) => TagMask::from_index(idx),
                        _ => None,
                    }
                })
                .unwrap_or(TagMask::EMPTY)
        }
    };

    let current_tagset = ctx.core().model().expect_selected_monitor().selected_tags();
    let is_current_tag = (initial_tag & ctx.core().model().tags.mask()) == current_tagset;
    let has_sel = ctx.core().model().selected_win().is_some();

    // Click on a *different* tag → switch view, no drag.
    if !is_current_tag && !initial_tag.is_empty() {
        crate::tags::view::view_tags(ctx, initial_tag);
        return false;
    }
    // No selected window → nothing to drag.
    if !has_sel {
        return false;
    }

    // Initialise the drag state machine.
    ctx.core_mut().drag_state_mut().tag = crate::core_state::TagDragState {
        active: true,
        initial_tag,
        monitor_id: selmon_id,
        mon_mx,
        last_tag: None,
        cursor_on_bar: true,
        last_motion: None,
        button: btn,
    };
    set_cursor_style(ctx, AltCursor::Move);
    ctx.core_mut().bar.hover.set(selmon_id, Gesture::None, true);
    ctx.request_bar_update();
    true
}

/// Process a single motion event during an active tag drag.
///
/// Updates gesture highlighting and detects when the cursor leaves the bar.
/// Returns `false` if the cursor left the bar (caller should finish the drag).
pub fn drag_tag_motion(ctx: &mut WmCtx, root: Point) -> bool {
    if !ctx.core().drag_state().tag.active {
        return false;
    }

    let selmon_id = ctx.core_mut().drag_state_mut().tag.monitor_id;
    let mon_mx = ctx.core_mut().drag_state_mut().tag.mon_mx;

    let bar_bottom = {
        let mon = ctx.core_mut().model_mut().expect_selected_monitor();
        mon.bar_y() + mon.bar_height + 1
    };

    if root.y > bar_bottom {
        ctx.core_mut().drag_state_mut().tag.cursor_on_bar = false;
        return false;
    }

    // Store last motion for release handling.  Modifier state is not available
    // from root coords alone; the caller sets it via drag_tag_finish.
    ctx.core_mut().drag_state_mut().tag.last_motion = Some((root, 0));

    let local_x = root.x - mon_mx;
    let new_gesture = {
        let core = ctx.core();
        core.state()
            .model
            .monitors
            .get(selmon_id)
            .map(|mon| mon.bar_position_at_x(core, local_x).to_gesture())
            .unwrap_or(Gesture::None)
    };
    let gesture_key = match new_gesture {
        Gesture::Tag(idx) => Some(idx),
        _ => None,
    };

    if ctx.core_mut().drag_state_mut().tag.last_tag != gesture_key {
        ctx.core_mut().drag_state_mut().tag.last_tag = gesture_key;
        ctx.core_mut().bar.hover.set(selmon_id, new_gesture, true);
        ctx.request_bar_update();
    }
    true
}

/// Finish a tag drag: apply the action based on the final position and
/// modifier keys held at release time.
///
/// `modifier_state` is the X11-style modifier bitmask at release time
/// (Shift, Control, …).
pub fn drag_tag_finish(ctx: &mut WmCtx, modifier_state: u32) {
    if !ctx.core().drag_state().tag.active {
        return;
    }

    let selmon_id = ctx.core_mut().drag_state_mut().tag.monitor_id;
    let cursor_on_bar = ctx.core_mut().drag_state_mut().tag.cursor_on_bar;
    let last_motion = ctx.core_mut().drag_state_mut().tag.last_motion;

    // Clear state first so re-entrant calls are safe.
    ctx.core_mut().drag_state_mut().tag.active = false;

    if cursor_on_bar && let Some((root, _)) = last_motion {
        let position = {
            let core = ctx.core();
            let mon = core.model().expect_selected_monitor();
            let local_x = root.x - mon.work_rect().x;
            mon.bar_position_at_x(core, local_x)
        };

        if let BarPosition::Tag(tag_idx) = position {
            let tag_mask = TagMask::from_index(tag_idx).unwrap_or(TagMask::EMPTY);
            if (modifier_state & SHIFT_MASK) != 0 {
                if let Some(win) = ctx
                    .core_mut()
                    .state_mut()
                    .monitor(selmon_id)
                    .and_then(|m| m.selected)
                {
                    crate::tags::client_tags::set_client_tag(ctx, win, tag_mask);
                }
            } else if (modifier_state & CTRL_MASK) != 0 {
                crate::tags::client_tags::tag_all(ctx, tag_mask);
            } else if let Some(win) = ctx
                .core_mut()
                .state_mut()
                .monitor(selmon_id)
                .and_then(|m| m.selected)
            {
                crate::tags::client_tags::follow_tag(ctx, win, tag_mask);
            }
        }
    }

    ctx.core_mut().bar.hover.clear();
    set_cursor_style(ctx, AltCursor::Default);
    ctx.request_bar_update();
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
    crate::backend::x11::grab::mouse_drag_loop(ctx, btn, AltCursor::Move, false, |ctx, event| {
        if let BackendEvent::Motion { root, modifiers } = event {
            // Store motion with modifier state for release handling.
            ctx.core.drag_state_mut().tag.last_motion = Some((*root, *modifiers));

            let mut wm_ctx = WmCtx::X11(ctx.reborrow());
            return drag_tag_motion(&mut wm_ctx, *root);
        }
        true
    });

    let modifier_state = {
        ctx.core
            .state()
            .drag
            .tag
            .last_motion
            .map(|(_, m)| m)
            .unwrap_or(0)
    };

    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    drag_tag_finish(&mut wm_ctx, modifier_state);
}
