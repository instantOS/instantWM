//! View (workspace) navigation.

use crate::backend::BackendKind;
use crate::contexts::WmCtx;
// focus() is used via focus_soft() in this module
use crate::layouts::arrange;
use crate::types::{Direction, TagMask, WindowId, SCRATCHPAD_MASK};
use x11rb::protocol::xproto::ConnectionExt;

/// View tags using type-safe mask.
pub fn view(ctx: &mut WmCtx, mask: TagMask) {
    let selmon_id = ctx.g.selmon_id();
    let tagmask = TagMask::from_bits(ctx.g.tags.mask());

    // Validate mask
    let effective_mask = mask & tagmask;
    if effective_mask.is_empty() {
        return;
    }

    // Get all needed state in one globals access
    let (_prev_tag, _current_tag) = {
        let mon = match ctx.g.selmon_mut() {
            Some(m) => m,
            None => return,
        };

        mon.seltags ^= 1;
        mon.tagset[mon.seltags as usize] = effective_mask.bits();

        let prev = mon.current_tag;

        if mask == TagMask::ALL_BITS {
            mon.current_tag = 0;
            (prev, 0)
        } else {
            let new_tag = effective_mask.first_tag().unwrap_or(0);
            if new_tag == mon.current_tag {
                mon.seltags ^= 1;
                return;
            }
            mon.prev_tag = prev;
            mon.current_tag = new_tag;
            (prev, new_tag)
        }
    };

    // Apply pertag settings and update
    apply_pertag_settings(ctx);
    crate::focus::focus_soft(ctx, None);
    arrange(ctx, Some(selmon_id));
}

/// Toggle view of tags using type-safe mask.
pub fn toggle_view(ctx: &mut WmCtx, mask: TagMask) {
    let selmon_id = ctx.g.selmon_id();
    let tagmask = TagMask::from_bits(ctx.g.tags.mask());

    let new_mask = ctx
        .g
        .selmon()
        .map(|m| TagMask::from_bits(m.tagset[m.seltags as usize]))
        .unwrap_or(TagMask::EMPTY)
        ^ (mask & tagmask);

    if new_mask.is_empty() {
        return;
    }

    let mon = match ctx.g.selmon_mut() {
        Some(m) => m,
        None => return,
    };

    mon.tagset[mon.seltags as usize] = new_mask.bits();

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

    apply_pertag_settings(ctx);
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

    let valid_mask = TagMask::from_bits(ctx.g.tags.mask());
    let clicked_mask = clicked_mask & valid_mask;
    if clicked_mask.is_empty() {
        return;
    }

    let current = ctx
        .g
        .selmon()
        .map(|m| TagMask::from_bits(m.tagset[m.seltags as usize]))
        .unwrap_or(TagMask::EMPTY);

    // If this is the only visible tag, removing it would leave nothing — bail.
    if current & valid_mask == clicked_mask {
        return;
    }

    // toggle_view XORs the mask in/out of the current tagset, which is
    // exactly add-if-absent / remove-if-present.
    toggle_view(ctx, clicked_mask);
}

pub fn shift_view(ctx: &mut WmCtx, direction: Direction) {
    let (tagset, numtags) = match ctx.g.selmon() {
        Some(mon) => (
            TagMask::from_bits(mon.tagset[mon.seltags as usize]),
            ctx.g.tags.count(),
        ),
        None => return,
    };

    let mut next_mask = tagset;
    let mut found = false;

    for step in 1..=10i32 {
        next_mask = match direction {
            Direction::Right | Direction::Down => tagset.rotate_left(step as usize, numtags),
            Direction::Left | Direction::Up => tagset.rotate_right(step as usize, numtags),
        };

        let mut cursor = ctx.g.selmon().and_then(|m| m.clients);

        while let Some(win) = cursor {
            match ctx.g.clients.get(&win) {
                Some(c) => {
                    if TagMask::from_bits(c.tags).intersects(next_mask) {
                        found = true;
                        break;
                    }
                    cursor = c.next;
                }
                None => break,
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

pub fn last_view(ctx: &mut WmCtx) {
    let (current_tag, prev_tag) = match ctx.g.selmon() {
        Some(mon) => (mon.current_tag, mon.prev_tag),
        None => return,
    };

    if current_tag == prev_tag {
        crate::focus::focus_last_client(ctx);
        return;
    }

    if let Some(mask) = TagMask::single(prev_tag) {
        view(ctx, mask);
    }
}

pub fn win_view(ctx: &mut WmCtx) {
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return;
    };

    let Ok(cookie) = conn.get_input_focus() else {
        return;
    };
    let reply = match cookie.reply() {
        Ok(r) => r,
        Err(_) => return,
    };
    let focused_win = WindowId::from(reply.focus);

    let client_win = find_client_for_window(ctx, focused_win);
    let Some(win) = client_win else { return };

    let tags = ctx.g.clients.get(&win).map(|c| c.tags).unwrap_or(1);

    let scratchpad = TagMask::from_bits(SCRATCHPAD_MASK);
    let tag_mask = TagMask::from_bits(tags);

    if tag_mask == scratchpad {
        let current_tag = ctx.g.selmon().map(|m| m.current_tag).unwrap_or(1);
        if let Some(mask) = TagMask::single(current_tag) {
            view(ctx, mask);
        }
    } else {
        view(ctx, tag_mask);
    }

    crate::focus::focus_soft(ctx, Some(win));
}

pub fn swap_tags(ctx: &mut WmCtx, mask: TagMask) {
    let selmon_id = ctx.g.selmon_id();
    let tagmask = TagMask::from_bits(ctx.g.tags.mask());
    let newtag = mask & tagmask;

    let (current_tag, current_tagset) = match ctx.g.selmon() {
        Some(mon) => (
            mon.current_tag,
            TagMask::from_bits(mon.tagset[mon.seltags as usize]),
        ),
        None => return,
    };

    // Can only swap from single-tag view
    if newtag == current_tagset || current_tagset.is_empty() || !current_tagset.is_single() {
        return;
    }

    let target_idx = newtag.first_tag().unwrap_or(0);

    let clients_to_swap: Vec<WindowId> = {
        let mut result = Vec::new();
        let mut cursor = ctx.g.selmon().and_then(|m| m.clients);

        while let Some(win) = cursor {
            match ctx.g.clients.get(&win) {
                Some(c) => {
                    let ctags = TagMask::from_bits(c.tags);
                    if ctags.intersects(newtag) || ctags.intersects(current_tagset) {
                        result.push(win);
                    }
                    cursor = c.next;
                }
                None => break,
            }
        }
        result
    };

    for win in clients_to_swap {
        if let Some(client) = ctx.g.clients.get_mut(&win) {
            let ctags = TagMask::from_bits(client.tags);
            let new_tags = ctags ^ current_tagset ^ newtag;
            client.tags = if new_tags.is_empty() {
                newtag.bits()
            } else {
                new_tags.bits()
            };
        }
    }

    if let Some(mon) = ctx.g.selmon_mut() {
        mon.tagset[mon.seltags as usize] = newtag.bits();
        if mon.prev_tag == target_idx {
            mon.prev_tag = current_tag;
        }
        mon.current_tag = target_idx;
    }

    crate::focus::focus_soft(ctx, None);
    arrange(ctx, Some(selmon_id));
}

pub fn follow_view(ctx: &mut WmCtx) {
    let selmon_id = ctx.g.selmon_id();
    let sel_win = ctx.g.selmon().and_then(|m| m.sel);
    let Some(win) = sel_win else { return };

    let prev_tag = match ctx.g.selmon() {
        Some(mon) => mon.prev_tag,
        None => return,
    };

    if prev_tag == 0 {
        return;
    }

    let target_mask = TagMask::single(prev_tag).unwrap_or(TagMask::EMPTY);

    if let Some(client) = ctx.g.clients.get_mut(&win) {
        client.tags = target_mask.bits();
    }

    view(ctx, target_mask);
    crate::focus::focus_soft(ctx, Some(win));
    arrange(ctx, Some(selmon_id));
}

pub fn toggle_overview(ctx: &mut WmCtx, _mask: TagMask) {
    let selmon_id = ctx.g.selmon_id();
    let (has_clients, current_tag, num_tags) = {
        let has_clients = ctx.g.selmon().map(|m| m.clients.is_some()).unwrap_or(false);
        let current_tag = ctx.g.selmon().map(|m| m.current_tag);
        (has_clients, current_tag, ctx.g.tags.count())
    };

    if !has_clients {
        if current_tag == Some(0) {
            last_view(ctx);
        }
        return;
    }

    match current_tag {
        Some(0) => {
            crate::floating::restore_all_floating(ctx, Some(selmon_id));
            win_view(ctx);
        }
        Some(_) => {
            crate::floating::save_all_floating(ctx, Some(selmon_id));
            let all_tags = TagMask::all(num_tags);
            view(ctx, all_tags);
        }
        None => {}
    }
}

pub fn toggle_fullscreen_overview(ctx: &mut WmCtx, _mask: TagMask) {
    let current_tag = ctx.g.selmon().map(|m| m.current_tag);

    match current_tag {
        Some(0) => win_view(ctx),
        Some(_) => {
            let num_tags = ctx.g.tags.count();
            view(ctx, TagMask::all(num_tags))
        }
        None => {}
    }
}

pub(super) fn apply_pertag_settings(ctx: &mut WmCtx) {
    let (nmaster, mfact) = {
        let Some(mon) = ctx.g.selmon() else {
            return;
        };
        let current_tag = mon.current_tag;
        if current_tag == 0 || current_tag > mon.tags.len() {
            return;
        }
        let tag = &mon.tags[current_tag - 1];
        (tag.nmaster, tag.mfact)
    };

    if let Some(mon) = ctx.g.selmon_mut() {
        mon.nmaster = nmaster;
        mon.mfact = mfact;
    }
}

pub fn scroll_view(ctx: &mut WmCtx, dir: Direction) {
    let selmon_id = ctx.g.selmon_id();
    let (current_tag, tagset, _tagmask) = match ctx.g.selmon() {
        Some(mon) => (
            mon.current_tag,
            TagMask::from_bits(mon.tagset[mon.seltags as usize]),
            TagMask::from_bits(ctx.g.tags.mask()),
        ),
        None => return,
    };

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

    if let Some(mon) = ctx.g.selmon_mut() {
        mon.seltags ^= 1;
        mon.tagset[mon.seltags as usize] = new_mask.bits();
        mon.prev_tag = mon.current_tag;
        mon.current_tag = new_mask.first_tag().unwrap_or(0);
    }
    apply_pertag_settings(ctx);
    crate::focus::focus_soft(ctx, None);
    arrange(ctx, Some(selmon_id));
}

fn find_client_for_window(ctx: &WmCtx, win: WindowId) -> Option<WindowId> {
    if ctx.g.clients.contains(&win) {
        return Some(win);
    }

    let mut cursor = ctx.g.selmon().and_then(|m| m.clients);

    while let Some(c_win) = cursor {
        match ctx.g.clients.get(&c_win) {
            Some(c) => {
                if c.win == win {
                    return Some(c_win);
                }
                cursor = c.next;
            }
            None => break,
        }
    }

    None
}
