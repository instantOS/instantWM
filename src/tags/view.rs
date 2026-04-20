//! View (workspace) navigation.

use crate::contexts::WmCtx;
use crate::types::{HorizontalDirection, MonitorId, TagMask, WindowId};

fn finalize_view_change(ctx: &mut WmCtx, selmon_id: MonitorId) {
    crate::focus::focus_soft(ctx, None);
    ctx.core_mut()
        .globals_mut()
        .queue_layout_for_monitor_urgent(selmon_id);
}

fn adjacent_scroll_mask(
    current_tag: usize,
    tagset: TagMask,
    dir: HorizontalDirection,
) -> Option<TagMask> {
    if !tagset.is_single() {
        return None;
    }

    let max_tag = crate::constants::animation::MAX_TAG_NUMBER as usize;
    let next_tag = match dir {
        HorizontalDirection::Left if current_tag > 1 => current_tag - 1,
        HorizontalDirection::Right if current_tag < max_tag => current_tag + 1,
        _ => return None,
    };

    TagMask::single(next_tag)
}

fn commit_view_selection(ctx: &mut WmCtx, new_mask: TagMask) -> Option<MonitorId> {
    let selmon_id = ctx.core().globals().selected_monitor_id();

    {
        let mon = ctx.core_mut().globals_mut().selected_monitor_mut();
        let previous_mask = mon.selected_tags();
        if previous_mask == new_mask {
            return None;
        }

        let previous_current_tag = mon.current_tag_index();
        mon.sel_tags ^= 1;
        mon.set_selected_tags(new_mask);

        if previous_current_tag != mon.current_tag_index()
            && let Some(previous_current_tag) = previous_current_tag
        {
            mon.prev_tag = Some(previous_current_tag);
        }
    }

    Some(selmon_id)
}

/// View tags using type-safe mask.
pub fn view_tags(ctx: &mut WmCtx, mask: TagMask) {
    let tagmask = ctx.core().globals().tags.mask();
    let effective_mask = mask & tagmask;
    if effective_mask.is_empty() {
        return;
    }

    let Some(selmon_id) = commit_view_selection(ctx, effective_mask) else {
        return;
    };

    finalize_view_change(ctx, selmon_id);
}

pub fn toggle_view_ctx(ctx: &mut WmCtx, mask: TagMask) {
    let tagmask = ctx.core().globals().tags.mask();
    let new_mask = ctx.core().globals().selected_monitor().selected_tags() ^ (mask & tagmask);
    if new_mask.is_empty() {
        return;
    }

    let Some(selmon_id) = commit_view_selection(ctx, new_mask) else {
        return;
    };

    finalize_view_change(ctx, selmon_id);
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

    let valid_mask = ctx.core().globals().tags.mask();
    let clicked_mask = clicked_mask & valid_mask;
    if clicked_mask.is_empty() {
        return;
    }

    let current = ctx.core().globals().selected_monitor().selected_tags();

    // If this is the only visible tag, removing it would leave nothing — bail.
    if current & valid_mask == clicked_mask {
        return;
    }

    // toggle_view XORs the mask in/out of the current tagset, which is
    // exactly add-if-absent / remove-if-present.
    toggle_view_ctx(ctx, clicked_mask);
}

pub fn shift_view(ctx: &mut WmCtx, direction: HorizontalDirection) {
    let mon = ctx.core().globals().selected_monitor();
    let (tagset, numtags) = (mon.selected_tags(), ctx.core().globals().tags.count());

    let mut next_mask = tagset;
    let mut found = false;

    for step in 1..=10i32 {
        next_mask = match direction {
            HorizontalDirection::Right => tagset.rotate_left(step as usize, numtags),
            HorizontalDirection::Left => tagset.rotate_right(step as usize, numtags),
        };

        let clients = ctx.core().globals().selected_monitor().clients.clone();

        for &win in &clients {
            if let Some(c) = ctx.core().client(win)
                && c.tags.intersects(next_mask)
            {
                found = true;
                break;
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
    let next_mask = next_mask & !TagMask::SCRATCHPAD;

    view_tags(ctx, next_mask);
}

pub fn last_view(ctx: &mut WmCtx) {
    let mon = ctx.core().globals().selected_monitor();
    let (current_tag, prev_tag) = (mon.current_tag_index(), mon.prev_tag);

    if current_tag == prev_tag {
        crate::focus::focus_last_client(ctx);
        return;
    }

    if let Some(mask) = prev_tag.and_then(TagMask::single) {
        view_tags(ctx, mask);
    }
}

pub fn win_view(ctx: &mut WmCtx) {
    let Some(win) = ctx.core().selected_client() else {
        return;
    };

    let tag_mask = ctx
        .core()
        .globals()
        .clients
        .tag_mask(win)
        .unwrap_or(TagMask::single(1).unwrap_or(TagMask::EMPTY));

    if tag_mask.is_scratchpad_only() {
        let current_tag = ctx.core().globals().selected_monitor().current_tag_index();
        if let Some(mask) = current_tag.and_then(TagMask::single) {
            view_tags(ctx, mask);
        }
    } else {
        view_tags(ctx, tag_mask);
    }

    crate::focus::focus_soft(ctx, Some(win));
}

pub fn swap_tags_ctx(ctx: &mut WmCtx, mask: TagMask) {
    let selmon_id = ctx.core().globals().selected_monitor_id();
    let tagmask = ctx.core().globals().tags.mask();
    let newtag = mask & tagmask;
    let mon = ctx.core().globals().selected_monitor();
    let (current_tag, current_tagset) = (mon.current_tag_index(), mon.selected_tags());
    if newtag == current_tagset || current_tagset.is_empty() || !current_tagset.is_single() {
        return;
    }
    let target_idx = newtag.first_tag().unwrap_or(0);
    let clients_to_swap: Vec<WindowId> = {
        let mut result = Vec::new();
        let m = ctx.core().globals().selected_monitor();
        for (win, c) in m.iter_clients(ctx.core().globals().clients.map()) {
            let ctags = c.tags;
            if ctags.intersects(newtag) || ctags.intersects(current_tagset) {
                result.push(win);
            }
        }
        result
    };
    for win in clients_to_swap {
        if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
            let ctags = client.tags;
            let new_tags = ctags ^ current_tagset ^ newtag;
            client.set_tag_mask(if new_tags.is_empty() {
                newtag
            } else {
                new_tags
            });
        }
    }
    let mon = ctx.core_mut().globals_mut().selected_monitor_mut();
    mon.set_selected_tags(newtag);
    if mon.prev_tag == Some(target_idx) {
        mon.prev_tag = current_tag;
    }
    crate::focus::focus_soft(ctx, None);
    ctx.core_mut()
        .globals_mut()
        .queue_layout_for_monitor_urgent(selmon_id);
}

pub fn follow_view(ctx: &mut WmCtx) {
    let selmon_id = ctx.core().globals().selected_monitor_id();
    let selected_window = ctx.core().selected_client();
    let Some(win) = selected_window else { return };

    let prev_tag = ctx.core().globals().selected_monitor().prev_tag;

    if prev_tag.is_none() {
        return;
    }

    let target_mask = prev_tag.and_then(TagMask::single).unwrap_or(TagMask::EMPTY);

    if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
        client.set_tag_mask(target_mask);
    }

    view_tags(ctx, target_mask);
    crate::focus::focus_soft(ctx, Some(win));
    ctx.core_mut()
        .globals_mut()
        .queue_layout_for_monitor_urgent(selmon_id);
}

pub fn toggle_overview(ctx: &mut WmCtx, _mask: TagMask) {
    if crate::overview::is_active(ctx.core()) {
        ctx.with_behavior_mut(|behavior| behavior.overview_accept_selection_on_exit = true);
        ctx.reset_mode();
        ctx.request_bar_update(None);
        return;
    }

    if ctx.core().globals().selected_monitor().clients.is_empty() {
        return;
    }

    ctx.set_current_mode(crate::overview::OVERVIEW_MODE_NAME.to_string());
    ctx.request_bar_update(None);
}

pub fn cancel_overview(ctx: &mut WmCtx, _mask: TagMask) {
    if !crate::overview::is_active(ctx.core()) {
        return;
    }

    ctx.with_behavior_mut(|behavior| behavior.overview_accept_selection_on_exit = false);
    ctx.reset_mode();
    ctx.request_bar_update(None);
}

#[cfg(test)]
mod tests {
    use super::adjacent_scroll_mask;
    use crate::types::{HorizontalDirection, TagMask};

    #[test]
    fn adjacent_scroll_mask_moves_left_and_right() {
        let current = 3;
        let tagset = TagMask::single(current).unwrap_or(TagMask::EMPTY);
        assert_eq!(
            adjacent_scroll_mask(current, tagset, HorizontalDirection::Left),
            TagMask::single(2)
        );
        assert_eq!(
            adjacent_scroll_mask(current, tagset, HorizontalDirection::Right),
            TagMask::single(4)
        );
    }

    #[test]
    fn adjacent_scroll_mask_requires_single_tag_and_bounds() {
        let multi = TagMask::single(2).unwrap_or(TagMask::EMPTY)
            | TagMask::single(3).unwrap_or(TagMask::EMPTY);
        assert_eq!(
            adjacent_scroll_mask(2, multi, HorizontalDirection::Left),
            None
        );
        assert_eq!(
            adjacent_scroll_mask(
                1,
                TagMask::single(1).unwrap_or(TagMask::EMPTY),
                HorizontalDirection::Left
            ),
            None
        );
    }
}

pub fn scroll_view(ctx: &mut WmCtx, dir: HorizontalDirection) {
    let mon = ctx.core().globals().selected_monitor();
    let (Some(current_tag), tagset) = (mon.current_tag_index(), mon.selected_tags()) else {
        return;
    };

    let Some(new_mask) = adjacent_scroll_mask(current_tag, tagset, dir) else {
        return;
    };

    let Some(selmon_id) = commit_view_selection(ctx, new_mask) else {
        return;
    };

    finalize_view_change(ctx, selmon_id);
}

/// Scroll to adjacent tag and return the affected monitor id.
pub fn scroll_view_for_slide(ctx: &mut WmCtx, dir: HorizontalDirection) -> Option<MonitorId> {
    let mon = ctx.core().globals().selected_monitor();
    let (Some(current_tag), tagset) = (mon.current_tag_index(), mon.selected_tags()) else {
        return None;
    };

    let new_mask = adjacent_scroll_mask(current_tag, tagset, dir)?;
    let selmon_id = commit_view_selection(ctx, new_mask)?;
    crate::focus::focus_soft(ctx, None);
    Some(selmon_id)
}
