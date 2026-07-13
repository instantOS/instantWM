//! View (workspace) navigation.

use crate::contexts::WmCtx;
use crate::types::{HorizontalDirection, MonitorId, TagMask, WindowId};

fn finalize_view_change(ctx: &mut WmCtx, selmon_id: MonitorId) {
    ctx.update_ewmh_desktop_props();
    crate::focus::focus(ctx, None);
    ctx.core_mut().queue_layout_for_monitor_urgent(selmon_id);
}

fn adjacent_scroll_mask(tagset: TagMask, dir: HorizontalDirection) -> Option<TagMask> {
    if !tagset.is_single() {
        return None;
    }

    let current_tag = tagset.first_tag()?;
    let max_tag = crate::constants::animation::MAX_TAG_NUMBER as usize;
    let next_tag = match dir {
        HorizontalDirection::Left if current_tag > 1 => current_tag - 1,
        HorizontalDirection::Right if current_tag < max_tag => current_tag + 1,
        _ => return None,
    };

    TagMask::single(next_tag)
}

pub(crate) fn commit_view_selection(
    monitors: &mut crate::monitor::MonitorManager,
    new_mask: TagMask,
) -> Option<MonitorId> {
    let selected_monitor_id = monitors.selected();
    let mon = monitors.selected_monitor_mut_unchecked();
    if mon.set_selected_tags_with_history(new_mask) {
        Some(selected_monitor_id)
    } else {
        None
    }
}

/// View tags using type-safe mask.
pub fn view_tags(ctx: &mut WmCtx, mask: TagMask) {
    let tagmask = ctx.core().model().tags.mask();
    let effective_mask = mask & tagmask;
    if effective_mask.is_empty() {
        return;
    }

    let g = ctx.core_mut().state_mut();
    let Some(selmon_id) = commit_view_selection(&mut g.model.monitors, effective_mask) else {
        return;
    };

    finalize_view_change(ctx, selmon_id);
}

pub fn toggle_view(ctx: &mut WmCtx, mask: TagMask) {
    let tagmask = ctx.core().model().tags.mask();
    let new_mask = ctx.core().model().selected_monitor().selected_tags() ^ (mask & tagmask);
    if new_mask.is_empty() {
        return;
    }

    let g = ctx.core_mut().state_mut();
    let Some(selmon_id) = commit_view_selection(&mut g.model.monitors, new_mask) else {
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
    // BarPosition uses 0-based indices; TagMask::from_index() handles the conversion.
    let clicked_mask = match TagMask::from_index(tag_idx) {
        Some(m) => m,
        None => return,
    };

    let valid_mask = ctx.core().model().tags.mask();
    let clicked_mask = clicked_mask & valid_mask;
    if clicked_mask.is_empty() {
        return;
    }

    let current = ctx.core().model().selected_monitor().selected_tags();

    // If this is the only visible tag, removing it would leave nothing — bail.
    if current & valid_mask == clicked_mask {
        return;
    }

    // toggle_view XORs the mask in/out of the current tagset, which is
    // exactly add-if-absent / remove-if-present.
    toggle_view(ctx, clicked_mask);
}

pub fn shift_view(ctx: &mut WmCtx, direction: HorizontalDirection) {
    let mon = ctx.core().model().selected_monitor();
    let (tagset, numtags) = (mon.selected_tags(), ctx.core().model().tags.count());

    let mut next_mask = tagset;
    let mut found = false;

    for step in 1..=10i32 {
        next_mask = match direction {
            HorizontalDirection::Right => tagset.rotate_left(step as usize, numtags),
            HorizontalDirection::Left => tagset.rotate_right(step as usize, numtags),
        };

        let clients = ctx.core().model().selected_monitor().clients.clone();

        for &win in &clients {
            if let Some(c) = ctx.core().model().clients.get(&win)
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
    let mon = ctx.core().model().selected_monitor();
    let (current_tag, prev_tag) = (mon.current_tag_number(), mon.prev_tag);

    if current_tag == prev_tag {
        crate::focus::focus_last_client(ctx);
        return;
    }

    if let Some(mask) = prev_tag.and_then(TagMask::single) {
        view_tags(ctx, mask);
    }
}

pub fn win_view(ctx: &mut WmCtx) {
    let Some(win) = ctx.core().model().selected_win() else {
        return;
    };

    let tag_mask = ctx
        .core()
        .state()
        .model
        .clients
        .tag_mask(win)
        .unwrap_or(TagMask::single(1).unwrap_or(TagMask::EMPTY));

    if tag_mask.is_scratchpad_only() {
        let current_tag = ctx.core().model().selected_monitor().current_tag_number();
        if let Some(mask) = current_tag.and_then(TagMask::single) {
            view_tags(ctx, mask);
        }
    } else {
        view_tags(ctx, tag_mask);
    }

    crate::focus::focus(ctx, Some(win));
}

pub fn swap_tags(ctx: &mut WmCtx, mask: TagMask) {
    let selmon_id = ctx.core().model().selected_monitor_id();
    let tagmask = ctx.core().model().tags.mask();
    let newtag = mask & tagmask;
    let mon = ctx.core().model().selected_monitor();
    let (current_tag, current_tagset) = (mon.current_tag_number(), mon.selected_tags());
    if newtag == current_tagset || current_tagset.is_empty() || !current_tagset.is_single() {
        return;
    }
    let target_idx = newtag.first_tag().unwrap_or(0);
    let clients_to_swap: Vec<WindowId> = {
        let mut result = Vec::new();
        let m = ctx.core().model().selected_monitor();
        for (win, c) in m.iter_clients(ctx.core().model().clients.map()) {
            let ctags = c.tags;
            if ctags.intersects(newtag) || ctags.intersects(current_tagset) {
                result.push(win);
            }
        }
        result
    };
    for win in clients_to_swap {
        if let Some(client) = ctx.core_mut().model_mut().clients.get_mut(&win) {
            let ctags = client.tags;
            let new_tags = ctags ^ current_tagset ^ newtag;
            client.set_tag_mask(if new_tags.is_empty() {
                newtag
            } else {
                new_tags
            });
        }
    }
    let mon = ctx.core_mut().model_mut().selected_monitor_mut();
    mon.set_selected_tags(newtag);
    if mon.prev_tag == Some(target_idx) {
        mon.prev_tag = current_tag;
    }
    crate::focus::focus(ctx, None);
    ctx.core_mut().queue_layout_for_monitor_urgent(selmon_id);
}

pub fn follow_view(ctx: &mut WmCtx) {
    let selmon_id = ctx.core().model().selected_monitor_id();
    let selected_window = ctx.core().model().selected_win();
    let Some(win) = selected_window else { return };

    let prev_tag = ctx.core().model().selected_monitor().prev_tag;

    if prev_tag.is_none() {
        return;
    }

    let target_mask = prev_tag.and_then(TagMask::single).unwrap_or(TagMask::EMPTY);

    if let Some(client) = ctx.core_mut().model_mut().clients.get_mut(&win) {
        client.set_tag_mask(target_mask);
    }

    view_tags(ctx, target_mask);
    crate::focus::focus(ctx, Some(win));
    ctx.core_mut().queue_layout_for_monitor_urgent(selmon_id);
}

#[cfg(test)]
mod tests {
    use super::adjacent_scroll_mask;
    use crate::types::{HorizontalDirection, TagMask};

    #[test]
    fn adjacent_scroll_mask_moves_left_and_right() {
        let tagset = TagMask::single(3).unwrap_or(TagMask::EMPTY);
        assert_eq!(
            adjacent_scroll_mask(tagset, HorizontalDirection::Left),
            TagMask::single(2)
        );
        assert_eq!(
            adjacent_scroll_mask(tagset, HorizontalDirection::Right),
            TagMask::single(4)
        );
    }

    #[test]
    fn adjacent_scroll_mask_requires_single_tag_and_bounds() {
        let multi = TagMask::single(2).unwrap_or(TagMask::EMPTY)
            | TagMask::single(3).unwrap_or(TagMask::EMPTY);
        assert_eq!(adjacent_scroll_mask(multi, HorizontalDirection::Left), None);
        assert_eq!(
            adjacent_scroll_mask(
                TagMask::single(1).unwrap_or(TagMask::EMPTY),
                HorizontalDirection::Left
            ),
            None
        );
    }
}

#[cfg(test)]
mod view_selection_tests {
    use super::commit_view_selection;
    use crate::core_state::CoreState;
    use crate::monitor::MonitorManager;
    use crate::types::*;

    fn make_globals_with_one_monitor(selected: TagMask) -> CoreState {
        let mut g = CoreState::default();
        let mut mmgr = MonitorManager::new();
        let mut mon = Monitor::default();
        mon.monitor_id = MonitorId::from_raw(0);
        mon.set_selected_tags(selected);
        mmgr.push(mon);
        mmgr.set_selected(MonitorId::from_raw(0));
        g.model.monitors = mmgr;
        g.model.tags.num_tags = 9;
        g
    }

    #[test]
    fn commit_view_changes_selection() {
        let tag1 = TagMask::single(1).unwrap();
        let tag2 = TagMask::single(2).unwrap();
        let mut g = make_globals_with_one_monitor(tag1);

        let result = commit_view_selection(&mut g.model.monitors, tag2);
        assert_eq!(result, Some(MonitorId::from_raw(0)));

        let mon = g.monitor(MonitorId::from_raw(0)).unwrap();
        assert_eq!(mon.selected_tags(), tag2);
    }

    #[test]
    fn commit_view_same_mask_returns_none() {
        let tag1 = TagMask::single(1).unwrap();
        let mut g = make_globals_with_one_monitor(tag1);

        let result = commit_view_selection(&mut g.model.monitors, tag1);
        assert!(result.is_none());
    }

    #[test]
    fn commit_view_updates_prev_tag() {
        let tag1 = TagMask::single(1).unwrap();
        let tag2 = TagMask::single(2).unwrap();
        let mut g = make_globals_with_one_monitor(tag1);

        // First change: tag1 -> tag2
        // tag1 (bit 0) has first_tag() = Some(1), tag2 (bit 1) has first_tag() = Some(2)
        // prev_tag should become Some(1) (the previous current_tag_number)
        let _ = commit_view_selection(&mut g.model.monitors, tag2);
        let mon = g.monitor(MonitorId::from_raw(0)).unwrap();
        assert_eq!(mon.prev_tag, Some(1));

        // Second change: tag2 -> tag3
        // prev_tag should become Some(2) (the previous current_tag_number)
        let tag3 = TagMask::single(3).unwrap();
        let _ = commit_view_selection(&mut g.model.monitors, tag3);
        let mon_after = g.monitor(MonitorId::from_raw(0)).unwrap();
        assert_eq!(mon_after.prev_tag, Some(2));
    }

    #[test]
    fn commit_view_no_prev_tag_when_stays_same_number() {
        // If the current tag number doesn't change, prev_tag should not update
        let multi_tag = TagMask::single(1).unwrap() | TagMask::single(2).unwrap();
        let tag3 = TagMask::single(3).unwrap();
        let mut g = make_globals_with_one_monitor(multi_tag);

        // multi-tag view -> single tag: should set prev tag since current_tag_number changes
        // prev_tag becomes Some(1) because the previous current_tag_number was... None
        // (multi-tag has no single current_tag_number)
        let result = commit_view_selection(&mut g.model.monitors, tag3);
        assert!(result.is_some());

        let mon = g.monitor(MonitorId::from_raw(0)).unwrap();
        // current_tag_number was None (multi-tag), now Some(3) (single tag 3)
        // Since previous_current_tag was None, the guard `let Some(previous_current_tag) = previous_current_tag`
        // fails, so prev_tag is NOT updated
        assert_eq!(mon.prev_tag, None);
    }
}

pub fn scroll_view(ctx: &mut WmCtx, dir: HorizontalDirection) {
    let tagset = ctx.core().model().selected_monitor().selected_tags();

    let Some(new_mask) = adjacent_scroll_mask(tagset, dir) else {
        return;
    };

    let g = ctx.core_mut().state_mut();
    let Some(selmon_id) = commit_view_selection(&mut g.model.monitors, new_mask) else {
        return;
    };

    finalize_view_change(ctx, selmon_id);
}

/// Scroll to adjacent tag and return the affected monitor id.
pub fn scroll_view_for_slide(ctx: &mut WmCtx, dir: HorizontalDirection) -> Option<MonitorId> {
    let tagset = ctx.core().model().selected_monitor().selected_tags();

    let new_mask = adjacent_scroll_mask(tagset, dir)?;
    let g = ctx.core_mut().state_mut();
    let selmon_id = commit_view_selection(&mut g.model.monitors, new_mask)?;
    crate::focus::focus(ctx, None);
    Some(selmon_id)
}
