//! View (workspace) navigation.

use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
// focus() is used via focus_soft() in this module
use crate::layouts::arrange;
use crate::types::{Direction, TagMask, WindowId, SCRATCHPAD_MASK};
use x11rb::protocol::xproto::ConnectionExt;

/// View tags using type-safe mask.
pub fn view(ctx: &mut WmCtx, mask: TagMask) {
    let selmon_id = ctx.g_mut().selected_monitor_id();
    let tagmask = TagMask::from_bits(ctx.g().tags.mask());
    let effective_mask = mask & tagmask;
    if effective_mask.is_empty() {
        return;
    }

    {
        let mon = ctx.g_mut().selected_monitor_mut();
        mon.sel_tags ^= 1;
        mon.set_selected_tags(effective_mask.bits());

        let prev = mon.current_tag;
        if mask == TagMask::ALL_BITS {
            mon.current_tag = 0;
        } else {
            let new_tag = effective_mask.first_tag().unwrap_or(0);
            if new_tag == mon.current_tag {
                mon.sel_tags ^= 1;
                return;
            }
            mon.prev_tag = prev;
            mon.current_tag = new_tag;
        }
    }

    apply_pertag_settings(ctx.core_mut());
    crate::focus::focus_soft(ctx, None);
    arrange(ctx, Some(selmon_id));
}

pub fn toggle_view_ctx(ctx: &mut WmCtx, mask: TagMask) {
    let selmon_id = ctx.g_mut().selected_monitor_id();
    let tagmask = TagMask::from_bits(ctx.g().tags.mask());
    let new_mask =
        TagMask::from_bits(ctx.g().selected_monitor().selected_tags()) ^ (mask & tagmask);
    if new_mask.is_empty() {
        return;
    }

    {
        let mon = ctx.g_mut().selected_monitor_mut();
        mon.set_selected_tags(new_mask.bits());
        if new_mask == TagMask::ALL_BITS {
            mon.prev_tag = mon.current_tag;
            mon.current_tag = 0;
        } else {
            let new_tag = new_mask.first_tag().unwrap_or(0);
            let current_tag = mon.current_tag;
            if current_tag == 0 || !new_mask.contains(current_tag) {
                mon.prev_tag = current_tag;
                mon.current_tag = new_tag;
            }
        }
    }

    apply_pertag_settings(ctx.core_mut());
    crate::focus::focus_soft(ctx, None);
    arrange(ctx, Some(selmon_id));
}

/// Toggle a single tag in or out of the current view by its 0-based index.
///
/// This is the handler for a right-click on a tag indicator in the bar.
/// The tag index comes directly from `BarPosition::Tag(idx)`, so no extra
/// lookup is needed.
///
/// Rules:
/// * If the clicked tag is the **only** tag currently visible, do nothing —
///   we never leave the user with an empty view.
/// * If the tag is **already** in the current view, remove it (toggle off).
/// * If the tag is **not** in the current view, add it (toggle on).
pub fn toggle_view_tag(ctx: &mut WmCtx, tag_idx: usize) {
    // BarPosition uses 0-based indices; TagMask::single() takes 1-based.
    let clicked_mask = match TagMask::single(tag_idx + 1) {
        Some(m) => m,
        None => return,
    };

    let valid_mask = TagMask::from_bits(ctx.g().tags.mask());
    let clicked_mask = clicked_mask & valid_mask;
    if clicked_mask.is_empty() {
        return;
    }

    let current = TagMask::from_bits(ctx.g().selected_monitor().selected_tags());

    // If this is the only visible tag, removing it would leave nothing — bail.
    if current & valid_mask == clicked_mask {
        return;
    }

    // toggle_view XORs the mask in/out of the current tagset, which is
    // exactly add-if-absent / remove-if-present.
    toggle_view_ctx(ctx, clicked_mask);
}

pub fn shift_view(ctx: &mut WmCtx, direction: Direction) {
    let mon = ctx.g().selected_monitor();
    let (tagset, numtags) = (
        TagMask::from_bits(mon.selected_tags()),
        ctx.g().tags.count(),
    );

    let mut next_mask = tagset;
    let mut found = false;

    for step in 1..=10i32 {
        next_mask = match direction {
            Direction::Right | Direction::Down => tagset.rotate_left(step as usize, numtags),
            Direction::Left | Direction::Up => tagset.rotate_right(step as usize, numtags),
        };

        let clients = ctx.g().selected_monitor().clients.clone();

        for &win in &clients {
            if let Some(c) = ctx.g().clients.get(&win) {
                if TagMask::from_bits(c.tags).intersects(next_mask) {
                    found = true;
                    break;
                }
            }
        }

        if found {
            break;
        }
    }

    if !found {
        return;
    }

    // Exclude scratchpad
    let scratchpad = TagMask::from_bits(SCRATCHPAD_MASK);
    let next_mask = next_mask & !scratchpad;

    view(ctx, next_mask);
}

pub fn last_view(ctx: &mut WmCtxX11) {
    let mon = ctx.core.g.selected_monitor();
    let (current_tag, prev_tag) = (mon.current_tag, mon.prev_tag);

    if current_tag == prev_tag {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        crate::focus::focus_last_client(&mut wm_ctx);
        return;
    }

    if let Some(mask) = TagMask::single(prev_tag) {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        view(&mut wm_ctx, mask);
    }
}

pub fn win_view(ctx: &mut WmCtxX11) {
    let conn = ctx.x11.conn;

    let Ok(cookie) = conn.get_input_focus() else {
        return;
    };
    let reply = match cookie.reply() {
        Ok(r) => r,
        Err(_) => return,
    };
    let focused_win = WindowId::from(reply.focus);

    let client_win = find_client_for_window(&ctx.core, focused_win);
    let Some(win) = client_win else { return };

    let tags = ctx.core.g.clients.get(&win).map(|c| c.tags).unwrap_or(1);

    let scratchpad = TagMask::from_bits(SCRATCHPAD_MASK);
    let tag_mask = TagMask::from_bits(tags);

    if tag_mask == scratchpad {
        let current_tag = ctx.core.g.selected_monitor().current_tag;
        if let Some(mask) = TagMask::single(current_tag) {
            let mut wm_ctx = WmCtx::X11(ctx.reborrow());
            view(&mut wm_ctx, mask);
        }
    } else {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        view(&mut wm_ctx, tag_mask);
    }

    crate::focus::focus_soft_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, Some(win));
}

pub fn swap_tags_ctx(ctx: &mut WmCtx, mask: TagMask) {
    let selmon_id = ctx.g().selected_monitor_id();
    let tagmask = TagMask::from_bits(ctx.g().tags.mask());
    let newtag = mask & tagmask;
    let mon = ctx.g().selected_monitor();
    let (current_tag, current_tagset) = (mon.current_tag, TagMask::from_bits(mon.selected_tags()));
    if newtag == current_tagset || current_tagset.is_empty() || !current_tagset.is_single() {
        return;
    }
    let target_idx = newtag.first_tag().unwrap_or(0);
    let clients_to_swap: Vec<WindowId> = {
        let mut result = Vec::new();
        let m = ctx.g().selected_monitor();
        for &win in &m.clients {
            if let Some(c) = ctx.g().clients.get(&win) {
                let ctags = TagMask::from_bits(c.tags);
                if ctags.intersects(newtag) || ctags.intersects(current_tagset) {
                    result.push(win);
                }
            }
        }
        result
    };
    for win in clients_to_swap {
        if let Some(client) = ctx.g_mut().clients.get_mut(&win) {
            let ctags = TagMask::from_bits(client.tags);
            let new_tags = ctags ^ current_tagset ^ newtag;
            client.tags = if new_tags.is_empty() {
                newtag.bits()
            } else {
                new_tags.bits()
            };
        }
    }
    let mon = ctx.g_mut().selected_monitor_mut();
    mon.set_selected_tags(newtag.bits());
    if mon.prev_tag == target_idx {
        mon.prev_tag = current_tag;
    }
    mon.current_tag = target_idx;
    crate::focus::focus_soft(ctx, None);
    arrange(ctx, Some(selmon_id));
}

pub fn follow_view(ctx: &mut WmCtxX11) {
    let selmon_id = ctx.core.g.selected_monitor_id();
    let selected_window = ctx.core.g.selected_monitor().sel;
    let Some(win) = selected_window else { return };

    let prev_tag = ctx.core.g.selected_monitor().prev_tag;

    if prev_tag == 0 {
        return;
    }

    let target_mask = TagMask::single(prev_tag).unwrap_or(TagMask::EMPTY);

    if let Some(client) = ctx.core.g.clients.get_mut(&win) {
        client.tags = target_mask.bits();
    }

    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    view(&mut wm_ctx, target_mask);
    crate::focus::focus_soft_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, Some(win));
    arrange(&mut WmCtx::X11(ctx.reborrow()), Some(selmon_id));
}

pub fn toggle_overview(ctx: &mut WmCtxX11, _mask: TagMask) {
    let selmon_id = ctx.core.g.selected_monitor_id();
    let (has_clients, current_tag, num_tags) = {
        let has_clients = !ctx.core.g.selected_monitor().clients.is_empty();
        let current_tag = ctx.core.g.selected_monitor().current_tag;
        (has_clients, current_tag, ctx.core.g.tags.count())
    };

    if !has_clients {
        if current_tag == 0 {
            last_view(ctx);
        }
        return;
    }

    match current_tag {
        0 => {
            crate::floating::restore_all_floating(&mut WmCtx::X11(ctx.reborrow()), Some(selmon_id));
            win_view(ctx);
        }
        _ => {
            crate::floating::save_all_floating(&mut WmCtx::X11(ctx.reborrow()), Some(selmon_id));
            let all_tags = TagMask::all(num_tags);
            let mut wm_ctx = WmCtx::X11(ctx.reborrow());
            view(&mut wm_ctx, all_tags);
        }
    }
}

pub fn toggle_fullscreen_overview(ctx: &mut WmCtxX11, _mask: TagMask) {
    let current_tag = ctx.core.g.selected_monitor().current_tag;

    match current_tag {
        0 => win_view(ctx),
        _ => {
            let num_tags = ctx.core.g.tags.count();
            let mut wm_ctx = WmCtx::X11(ctx.reborrow());
            view(&mut wm_ctx, TagMask::all(num_tags))
        }
    }
}

pub(super) fn apply_pertag_settings(core: &mut CoreCtx) {
    let (nmaster, mfact) = {
        let mon = core.g.selected_monitor();
        let current_tag = mon.current_tag;
        if current_tag == 0 || current_tag > mon.tags.len() {
            return;
        }
        let tag = &mon.tags[current_tag - 1];
        (tag.nmaster, tag.mfact)
    };

    let mon = core.g.selected_monitor_mut();
    mon.nmaster = nmaster;
    mon.mfact = mfact;
}

pub fn scroll_view(ctx: &mut WmCtx, dir: Direction) {
    let core = ctx.core_mut();
    let selmon_id = core.g.selected_monitor_id();
    let mon = core.g.selected_monitor();
    let (current_tag, tagset, _tagmask) = (
        mon.current_tag,
        TagMask::from_bits(mon.selected_tags()),
        TagMask::from_bits(core.g.tags.mask()),
    );

    if dir == Direction::Left && current_tag <= 1 {
        return;
    }
    if dir == Direction::Right
        && current_tag >= crate::constants::animation::MAX_TAG_NUMBER as usize
    {
        return;
    }

    if !tagset.is_single() {
        return;
    }

    let new_mask = match dir {
        Direction::Left => {
            if current_tag <= 1 {
                return;
            }
            TagMask::single(current_tag - 1).unwrap_or(TagMask::EMPTY)
        }
        Direction::Right => {
            if current_tag >= crate::constants::animation::MAX_TAG_NUMBER as usize {
                return;
            }
            TagMask::single(current_tag + 1).unwrap_or(TagMask::EMPTY)
        }
        Direction::Up => {
            if current_tag <= 1 {
                return;
            }
            TagMask::single(current_tag - 1).unwrap_or(TagMask::EMPTY)
        }
        Direction::Down => {
            if current_tag >= crate::constants::animation::MAX_TAG_NUMBER as usize {
                return;
            }
            TagMask::single(current_tag + 1).unwrap_or(TagMask::EMPTY)
        }
    };

    if new_mask.is_empty() {
        return;
    }

    // Get mutable access to monitor and apply changes
    {
        let mon = core.g.selected_monitor_mut();
        mon.sel_tags ^= 1;
        mon.set_selected_tags(new_mask.bits());
        mon.prev_tag = mon.current_tag;
        mon.current_tag = new_mask.first_tag().unwrap_or(0);
    }
    apply_pertag_settings(core);
    // Release core borrow before calling ctx methods
    let _ = core;
    crate::focus::focus_soft(ctx, None);
    arrange(ctx, Some(selmon_id));
}

fn find_client_for_window(core: &CoreCtx, win: WindowId) -> Option<WindowId> {
    if core.g.clients.contains(&win) {
        return Some(win);
    }

    let m = core.g.selected_monitor();
    for &c_win in &m.clients {
        if let Some(c) = core.g.clients.get(&c_win) {
            if c.win == win {
                return Some(c_win);
            }
        }
    }

    None
}
