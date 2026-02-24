//! View (workspace) navigation.

use crate::floating::{restore_all_floating, save_all_floating};
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::layouts::arrange;
use crate::types::{Direction, TagMask, SCRATCHPAD_MASK};
use crate::util::get_sel_win;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::Window;

/// View tags using type-safe mask.
pub fn view(mask: TagMask) {
    let selmon_id = get_globals().selmon;
    let tagmask = TagMask::from_bits(get_globals().tags.mask());

    // Validate mask
    let effective_mask = mask & tagmask;
    if effective_mask.is_empty() {
        return;
    }

    // Get all needed state in one globals access
    let (prev_tag, current_tag) = {
        let globals = get_globals_mut();
        let mon = match globals.monitors.get_mut(selmon_id) {
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
    let globals = get_globals_mut();
    apply_pertag_settings(globals);
    focus(None);
    arrange(Some(selmon_id));
}

/// Toggle view of tags using type-safe mask.
pub fn toggle_view(mask: TagMask) {
    let selmon_id = get_globals().selmon;
    let tagmask = TagMask::from_bits(get_globals().tags.mask());

    let new_mask = {
        let globals = get_globals();
        let current = globals
            .monitors
            .get(selmon_id)
            .map(|m| TagMask::from_bits(m.tagset[m.seltags as usize]))
            .unwrap_or(TagMask::EMPTY);
        current ^ (mask & tagmask)
    };

    if new_mask.is_empty() {
        return;
    }

    let globals = get_globals_mut();
    let mon = match globals.monitors.get_mut(selmon_id) {
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

    apply_pertag_settings(globals);
    focus(None);
    arrange(Some(selmon_id));
}

pub fn view_to_left() {
    scroll_view(Direction::Left);
}

pub fn view_to_right() {
    scroll_view(Direction::Right);
}

pub fn shift_view(direction: Direction) {
    let selmon_id = get_globals().selmon;
    let (tagset, numtags) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(selmon_id) else {
            return;
        };
        (TagMask::from_bits(mon.tagset[mon.seltags as usize]), globals.tags.count())
    };

    let mut next_mask = tagset;
    let mut found = false;

    for step in 1..=10i32 {
        next_mask = match direction {
            Direction::Right | Direction::Down => tagset.rotate_left(step as usize, numtags),
            Direction::Left | Direction::Up => tagset.rotate_right(step as usize, numtags),
        };

        let globals = get_globals();
        let mut cursor = globals.monitors.get(selmon_id).and_then(|m| m.clients);

        while let Some(win) = cursor {
            match globals.clients.get(&win) {
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

    view(next_mask);
}

pub fn last_view() {
    let selmon_id = get_globals().selmon;
    let (current_tag, prev_tag) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(selmon_id) else {
            return;
        };
        (mon.current_tag, mon.prev_tag)
    };

    if current_tag == prev_tag {
        crate::focus::focus_last_client();
        return;
    }

    if let Some(mask) = TagMask::single(prev_tag) {
        view(mask);
    }
}

pub fn win_view() {
    let selmon_id = get_globals().selmon;
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    let Ok(cookie) = conn.get_input_focus() else {
        return;
    };
    let reply = match cookie.reply() {
        Ok(r) => r,
        Err(_) => return,
    };
    let focused_win = reply.focus;

    let client_win = find_client_for_window(focused_win);
    let Some(win) = client_win else { return };

    let tags = {
        let globals = get_globals();
        globals.clients.get(&win).map(|c| c.tags).unwrap_or(1)
    };

    let scratchpad = TagMask::from_bits(SCRATCHPAD_MASK);
    let tag_mask = TagMask::from_bits(tags);

    if tag_mask == scratchpad {
        let current_tag = {
            let globals = get_globals();
            globals
                .monitors
                .get(selmon_id)
                .map(|m| m.current_tag)
                .unwrap_or(1)
        };
        if let Some(mask) = TagMask::single(current_tag) {
            view(mask);
        }
    } else {
        view(tag_mask);
    }

    focus(Some(win));
}

pub fn swap_tags(mask: TagMask) {
    let selmon_id = get_globals().selmon;
    let tagmask = TagMask::from_bits(get_globals().tags.mask());
    let newtag = mask & tagmask;

    let (current_tag, current_tagset) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(selmon_id) else {
            return;
        };
        (mon.current_tag, TagMask::from_bits(mon.tagset[mon.seltags as usize]))
    };

    // Can only swap from single-tag view
    if newtag == current_tagset || current_tagset.is_empty() || !current_tagset.is_single() {
        return;
    }

    let target_idx = newtag.first_tag().unwrap_or(0);

    let clients_to_swap: Vec<Window> = {
        let globals = get_globals();
        let mut result = Vec::new();
        let mut cursor = globals.monitors.get(selmon_id).and_then(|m| m.clients);

        while let Some(win) = cursor {
            match globals.clients.get(&win) {
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
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            let ctags = TagMask::from_bits(client.tags);
            let new_tags = ctags ^ current_tagset ^ newtag;
            client.tags = if new_tags.is_empty() { newtag.bits() } else { new_tags.bits() };
        }
    }

    let globals = get_globals_mut();
    if let Some(mon) = globals.monitors.get_mut(selmon_id) {
        mon.tagset[mon.seltags as usize] = newtag.bits();
        if mon.prev_tag == target_idx {
            mon.prev_tag = current_tag;
        }
        mon.current_tag = target_idx;
    }

    focus(None);
    arrange(Some(selmon_id));
}

pub fn follow_view() {
    let selmon_id = get_globals().selmon;
    let sel_win = get_sel_win();
    let Some(win) = sel_win else { return };

    let prev_tag = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(selmon_id) else {
            return;
        };
        mon.prev_tag
    };

    if prev_tag == 0 {
        return;
    }

    let target_mask = TagMask::single(prev_tag).unwrap_or(TagMask::EMPTY);

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.tags = target_mask.bits();
        }
    }

    view(target_mask);
    focus(Some(win));
    arrange(Some(selmon_id));
}

pub fn toggle_overview(mask: TagMask) {
    let selmon_id = get_globals().selmon;
    let (has_clients, current_tag, num_tags) = {
        let globals = get_globals();
        let has_clients = globals
            .monitors
            .get(selmon_id)
            .map(|m| m.clients.is_some())
            .unwrap_or(false);
        let current_tag = globals.monitors.get(selmon_id).map(|m| m.current_tag);
        (has_clients, current_tag, globals.tags.count())
    };

    if !has_clients {
        if current_tag == Some(0) {
            last_view();
        }
        return;
    }

    match current_tag {
        Some(0) => {
            restore_all_floating(Some(selmon_id));
            win_view();
        }
        Some(_) => {
            save_all_floating(Some(selmon_id));
            let all_tags = TagMask::all(num_tags);
            view(all_tags);
        }
        None => {}
    }
}

pub fn toggle_fullscreen_overview(mask: TagMask) {
    let selmon_id = get_globals().selmon;
    let current_tag = {
        let globals = get_globals();
        globals.monitors.get(selmon_id).map(|m| m.current_tag)
    };

    match current_tag {
        Some(0) => win_view(),
        Some(_) => {
            let num_tags = get_globals().tags.count();
            view(TagMask::all(num_tags))
        }
        None => {}
    }
}

pub(super) fn apply_pertag_settings(globals: &mut crate::globals::Globals) {
    let sel_mon_id = globals.selmon;

    let (nmaster, mfact) = {
        let Some(mon) = globals.monitors.get(sel_mon_id) else {
            return;
        };
        let current_tag = mon.current_tag;
        if current_tag == 0 || current_tag > globals.tags.tags.len() {
            return;
        }
        let tag = &globals.tags.tags[current_tag - 1];
        (tag.nmaster, tag.mfact)
    };

    if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
        mon.nmaster = nmaster;
        mon.mfact = mfact;
    }
}

fn scroll_view(dir: Direction) {
    let selmon_id = get_globals().selmon;
    let (current_tag, tagset, tagmask) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(selmon_id) else {
            return;
        };
        (
            mon.current_tag,
            TagMask::from_bits(mon.tagset[mon.seltags as usize]),
            TagMask::from_bits(globals.tags.mask()),
        )
    };

    if dir == Direction::Left && current_tag <= 1 {
        return;
    }
    if dir == Direction::Right && current_tag >= crate::constants::animation::MAX_TAG_NUMBER as usize {
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

    let globals = get_globals_mut();
    if let Some(mon) = globals.monitors.get_mut(selmon_id) {
        mon.seltags ^= 1;
        mon.tagset[mon.seltags as usize] = new_mask.bits();
        mon.prev_tag = mon.current_tag;
        mon.current_tag = new_mask.first_tag().unwrap_or(0);
    }
    apply_pertag_settings(globals);

    focus(None);
    arrange(Some(selmon_id));
}

fn find_client_for_window(win: Window) -> Option<Window> {
    let globals = get_globals();

    if globals.clients.contains_key(&win) {
        return Some(win);
    }

    let mut cursor = globals.monitors.get(globals.selmon).and_then(|m| m.clients);

    while let Some(c_win) = cursor {
        match globals.clients.get(&c_win) {
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
